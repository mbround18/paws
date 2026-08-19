//! Normalizes CI-provider context (repo, commit sha, ref, token) behind one
//! type, so subcommands that need it (e.g. `paws semver --push`) don't
//! hand-read `GITHUB_*`/`GITLAB_*` env vars directly at each call site.
//!
//! GitHub Actions is the only implemented provider today. [`Provider`] and
//! [`CiContext::detect`] are shaped so a GitLab CI (or other) provider can be
//! added later as another `detect`/`push_tag` branch, without changing any
//! call site that already holds a [`CiContext`].

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    GitHub,
}

/// Normalized CI context: which provider is running this, and the
/// repository/commit/token it gave us.
#[derive(Debug, Clone)]
pub struct CiContext {
    pub provider: Provider,
    pub owner: String,
    pub repo: String,
    /// Commit SHA being built. Empty outside CI (falls back to `""`, matching
    /// every other subcommand's existing `GITHUB_SHA` handling).
    pub sha: String,
    /// Verbatim ref, e.g. `refs/heads/main` or `refs/tags/v1.2.3`.
    pub git_ref: Option<String>,
    pub token: String,
}

impl CiContext {
    /// Detects the running CI provider from its environment variables.
    /// GitHub Actions only for now (`GITHUB_REPOSITORY` present); other
    /// providers add another `if let Ok(..) = std::env::var(..)` branch here.
    pub fn detect() -> Result<Self> {
        if let Ok(repository) = std::env::var("GITHUB_REPOSITORY") {
            let (owner, repo) = repository.split_once('/').with_context(|| {
                format!("GITHUB_REPOSITORY {repository:?} is not \"owner/repo\"")
            })?;
            let token = std::env::var("GITHUB_TOKEN")
                .or_else(|_| std::env::var("GH_TOKEN"))
                .context("GITHUB_TOKEN or GH_TOKEN must be set")?;
            return Ok(Self {
                provider: Provider::GitHub,
                owner: owner.to_string(),
                repo: repo.to_string(),
                sha: std::env::var("GITHUB_SHA").unwrap_or_default(),
                git_ref: std::env::var("GITHUB_REF").ok(),
                token,
            });
        }

        bail!("no supported CI provider detected (checked: GitHub Actions via GITHUB_REPOSITORY)")
    }
}

/// Identity a pushed tag is attributed to (GitHub's annotated-tag `tagger`
/// field). Defaults to a `paws`-branded bot identity distinct from whatever
/// token/app actually authenticates the API call.
#[derive(Debug, Clone)]
pub struct TagAuthor<'a> {
    pub name: &'a str,
    pub email: &'a str,
}

impl Default for TagAuthor<'static> {
    fn default() -> Self {
        Self {
            name: "paws-bot",
            email: "paws-bot@users.noreply.github.com",
        }
    }
}

/// Creates an annotated tag pointing at `ctx.sha` and pushes it — i.e. the
/// API equivalent of `git tag -a <tag> -m <tag> && git push origin <tag>`,
/// with no local git identity/worktree required (matches
/// `paws-release::GitHubReleaseClient`'s Contents-API-over-git precedent).
pub async fn push_tag(ctx: &CiContext, tag: &str, author: &TagAuthor<'_>) -> Result<()> {
    match ctx.provider {
        Provider::GitHub => push_tag_github(ctx, tag, author).await,
    }
}

async fn push_tag_github(ctx: &CiContext, tag: &str, author: &TagAuthor<'_>) -> Result<()> {
    let client = reqwest::Client::new();
    let base = format!(
        "https://api.github.com/repos/{}/{}/git",
        ctx.owner, ctx.repo
    );

    let tag_object: serde_json::Value = client
        .post(format!("{base}/tags"))
        .bearer_auth(&ctx.token)
        .header("User-Agent", "paws-environment")
        .json(&serde_json::json!({
            "tag": tag,
            "message": tag,
            "object": ctx.sha,
            "type": "commit",
            "tagger": { "name": author.name, "email": author.email },
        }))
        .send()
        .await
        .context("failed to reach GitHub's git/tags API")?
        .error_for_status()
        .context("GitHub rejected the tag object")?
        .json()
        .await
        .context("failed to parse GitHub's tag-object response")?;

    let tag_sha = tag_object
        .get("sha")
        .and_then(|v| v.as_str())
        .context("GitHub's tag-object response had no \"sha\" field")?;

    client
        .post(format!("{base}/refs"))
        .bearer_auth(&ctx.token)
        .header("User-Agent", "paws-environment")
        .json(&serde_json::json!({
            "ref": format!("refs/tags/{tag}"),
            "sha": tag_sha,
        }))
        .send()
        .await
        .context("failed to reach GitHub's git/refs API")?
        .error_for_status()
        .context("GitHub rejected the tag ref")?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tag_author_is_paws_bot() {
        let author = TagAuthor::default();
        assert_eq!(author.name, "paws-bot");
        assert_eq!(author.email, "paws-bot@users.noreply.github.com");
    }

    #[test]
    fn detect_fails_loudly_with_no_ci_env_vars_present() {
        // SAFETY: test-only env mutation, no other test in this crate reads
        // GITHUB_REPOSITORY concurrently.
        unsafe {
            std::env::remove_var("GITHUB_REPOSITORY");
        }
        let err = CiContext::detect().unwrap_err();
        assert!(err.to_string().contains("no supported CI provider"));
    }
}
