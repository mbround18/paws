//! Normalizes CI-provider context (repo, commit sha, ref, token) behind one
//! type, so subcommands that need it (e.g. `paws semver --push`) don't
//! hand-read `GITHUB_*`/`GITLAB_*` env vars directly at each call site.
//!
//! GitHub Actions is the only implemented provider today. [`Provider`] and
//! [`CiContext::detect`] are shaped so a GitLab CI (or other) provider can be
//! added later as another `detect`/`push_tag` branch, without changing any
//! call site that already holds a [`CiContext`].

use anyhow::{Context, Result, bail};

/// Credentials for a GitHub App, used to mint a short-lived installation
/// access token natively (no `actions/create-github-app-token` needed) —
/// see [`mint_github_app_installation_token`].
pub struct GitHubAppCredentials {
    /// The App's Client ID (the `Iv23...`-style string) — GitHub's docs
    /// recommend this over the legacy numeric App ID for the JWT `iss`
    /// claim.
    pub client_id: String,
    /// The App's private key, PEM-encoded.
    pub private_key_pem: String,
}

#[derive(serde::Serialize)]
struct AppJwtClaims {
    iat: i64,
    exp: i64,
    iss: String,
}

/// Mints a short-lived (~1 hour) installation access token for `owner/repo`,
/// authenticating as the GitHub App identified by `creds` — the same
/// mechanism `actions/create-github-app-token` uses, done natively so
/// `paws` doesn't need that Action as a separate CI step. A commit made
/// with this token registers as authored by the App, which matters when a
/// branch ruleset's required-status-check rule only bypasses for specific
/// actors (like this App) rather than the default `GITHUB_TOKEN`.
pub async fn mint_github_app_installation_token(
    creds: &GitHubAppCredentials,
    owner: &str,
    repo: &str,
) -> Result<String> {
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs() as i64;
    let claims = AppJwtClaims {
        // 60s in the past to tolerate clock drift between this machine and
        // GitHub's, per GitHub's own JWT-generation guidance.
        iat: now - 60,
        // GitHub caps this at 10 minutes past `iat`; 9 minutes leaves
        // margin without cutting it close.
        exp: now + 9 * 60,
        iss: creds.client_id.clone(),
    };
    let key = EncodingKey::from_rsa_pem(creds.private_key_pem.as_bytes())
        .context("GitHub App private key is not valid PEM")?;
    let jwt = encode(&Header::new(Algorithm::RS256), &claims, &key)
        .context("failed to sign the GitHub App JWT")?;

    let client = reqwest::Client::new();
    let auth_headers = |builder: reqwest::RequestBuilder| {
        builder
            .bearer_auth(&jwt)
            .header("User-Agent", "paws-environment")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
    };

    let installation: serde_json::Value = auth_headers(client.get(format!(
        "https://api.github.com/repos/{owner}/{repo}/installation"
    )))
    .send()
    .await
    .context("failed to reach GitHub's repository-installation API")?
    .error_for_status()
    .context(
        "GitHub rejected the repository-installation lookup — is the GitHub App actually \
         installed on this repository?",
    )?
    .json()
    .await
    .context("failed to parse GitHub's repository-installation response")?;
    let installation_id = installation
        .get("id")
        .and_then(|v| v.as_u64())
        .context("GitHub's repository-installation response had no \"id\" field")?;

    let access_token: serde_json::Value = auth_headers(client.post(format!(
        "https://api.github.com/app/installations/{installation_id}/access_tokens"
    )))
    .send()
    .await
    .context("failed to reach GitHub's installation access-tokens API")?
    .error_for_status()
    .context("GitHub rejected the installation access-token request")?
    .json()
    .await
    .context("failed to parse GitHub's installation access-token response")?;
    access_token
        .get("token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .context("GitHub's installation access-token response had no \"token\" field")
}

/// Resolves a GitHub token for `owner/repo`: mints a fresh GitHub App
/// installation token when `$GH_APP_CLIENT_ID` and a private key
/// (`$GH_APP_PRIVATE_KEY` or `$GH_APP_PRIVATE_KEY_FILE`) are both present,
/// otherwise falls back to the plain `$GITHUB_TOKEN`/`$GH_TOKEN` env vars
/// every call site used to read directly. Incomplete App credentials (a
/// client ID with no private key) fall back too, rather than half-attempting
/// App auth — a partially-configured App shouldn't shadow a working
/// `GITHUB_TOKEN`.
pub async fn resolve_github_token(owner: &str, repo: &str) -> Result<String> {
    if let Ok(client_id) = std::env::var("GH_APP_CLIENT_ID") {
        let private_key_pem = if let Ok(pem) = std::env::var("GH_APP_PRIVATE_KEY") {
            Some(pem)
        } else if let Ok(path) = std::env::var("GH_APP_PRIVATE_KEY_FILE") {
            Some(
                std::fs::read_to_string(&path)
                    .with_context(|| format!("failed to read $GH_APP_PRIVATE_KEY_FILE ({path})"))?,
            )
        } else {
            None
        };
        if let Some(private_key_pem) = private_key_pem {
            let creds = GitHubAppCredentials {
                client_id,
                private_key_pem,
            };
            return mint_github_app_installation_token(&creds, owner, repo).await;
        }
    }

    std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .context(
            "GITHUB_TOKEN (or GH_TOKEN) must be set — or $GH_APP_CLIENT_ID plus \
             $GH_APP_PRIVATE_KEY/$GH_APP_PRIVATE_KEY_FILE for GitHub App auth",
        )
}

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
    ///
    /// The token comes from [`resolve_github_token`] — a GitHub App
    /// installation token when `$GH_APP_CLIENT_ID`+a private key are
    /// configured, otherwise the plain `$GITHUB_TOKEN`/`$GH_TOKEN` env vars.
    pub async fn detect() -> Result<Self> {
        if let Ok(repository) = std::env::var("GITHUB_REPOSITORY") {
            let (owner, repo) = repository.split_once('/').with_context(|| {
                format!("GITHUB_REPOSITORY {repository:?} is not \"owner/repo\"")
            })?;
            let token = resolve_github_token(owner, repo).await?;
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

/// Creates an annotated tag pointing at `ctx.sha` and pushes it, then
/// creates the matching GitHub Release (`gh-reusable`'s original
/// `tagger.yaml` did both — `git tag && git push` followed by `gh release
/// create --generate-notes` — so a bare tag push alone would be an
/// incomplete replacement). No local git identity/worktree required
/// (matches `paws-release::GitHubReleaseClient`'s Contents-API-over-git
/// precedent). If the release creation fails after the tag already landed,
/// that error is still returned — the tag itself is not rolled back, since
/// `git push`-ing a tag isn't an atomic pair with creating a Release either
/// in the system this replaces.
pub async fn push_tag(ctx: &CiContext, tag: &str, author: &TagAuthor<'_>) -> Result<()> {
    match ctx.provider {
        Provider::GitHub => {
            push_tag_github(ctx, tag, author).await?;
            create_release_github(ctx, tag).await
        }
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

/// Creates a GitHub Release for an already-pushed tag, with auto-generated
/// notes — matches `gh-reusable`'s `tagger.yaml` (`gh release create "$TAG"
/// --title "Release $TAG" --generate-notes`) exactly, including the
/// `Release {tag}` title convention.
async fn create_release_github(ctx: &CiContext, tag: &str) -> Result<()> {
    reqwest::Client::new()
        .post(format!(
            "https://api.github.com/repos/{}/{}/releases",
            ctx.owner, ctx.repo
        ))
        .bearer_auth(&ctx.token)
        .header("User-Agent", "paws-environment")
        .json(&serde_json::json!({
            "tag_name": tag,
            "name": format!("Release {tag}"),
            "generate_release_notes": true,
        }))
        .send()
        .await
        .context("failed to reach GitHub's releases API")?
        .error_for_status()
        .context("GitHub rejected the release")?;

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

    // Serialized: every test below mutates process-wide env vars
    // (GITHUB_REPOSITORY/GH_APP_*/GITHUB_TOKEN/GH_TOKEN), and `cargo test`
    // runs test fns in this module concurrently on separate threads by
    // default — without this, one test's env mutation can leak into
    // another's assertions.
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// SAFETY: guarded by `ENV_LOCK` above — no concurrent env access from
    /// other tests in this module while a guard is held.
    unsafe fn clear_env(vars: &[&str]) {
        for var in vars {
            unsafe {
                std::env::remove_var(var);
            }
        }
    }

    const ENV_VARS_UNDER_TEST: &[&str] = &[
        "GITHUB_REPOSITORY",
        "GH_APP_CLIENT_ID",
        "GH_APP_PRIVATE_KEY",
        "GH_APP_PRIVATE_KEY_FILE",
        "GITHUB_TOKEN",
        "GH_TOKEN",
    ];

    #[tokio::test]
    async fn detect_fails_loudly_with_no_ci_env_vars_present() {
        let _guard = ENV_LOCK.lock().await;
        unsafe {
            clear_env(ENV_VARS_UNDER_TEST);
        }
        let err = CiContext::detect().await.unwrap_err();
        assert!(err.to_string().contains("no supported CI provider"));
    }

    #[tokio::test]
    async fn resolve_github_token_falls_back_to_plain_env_token_with_no_app_creds() {
        let _guard = ENV_LOCK.lock().await;
        unsafe {
            clear_env(ENV_VARS_UNDER_TEST);
            std::env::set_var("GITHUB_TOKEN", "plain-token-value");
        }
        let token = resolve_github_token("owner", "repo").await.unwrap();
        assert_eq!(token, "plain-token-value");
        unsafe {
            clear_env(ENV_VARS_UNDER_TEST);
        }
    }

    #[tokio::test]
    async fn resolve_github_token_ignores_an_incomplete_app_client_id_and_falls_back() {
        let _guard = ENV_LOCK.lock().await;
        unsafe {
            clear_env(ENV_VARS_UNDER_TEST);
            // A client ID with no private key anywhere shouldn't attempt
            // (and fail) App auth — it should fall back to GITHUB_TOKEN.
            std::env::set_var("GH_APP_CLIENT_ID", "Iv23liSomeClientId");
            std::env::set_var("GH_TOKEN", "fallback-token-value");
        }
        let token = resolve_github_token("owner", "repo").await.unwrap();
        assert_eq!(token, "fallback-token-value");
        unsafe {
            clear_env(ENV_VARS_UNDER_TEST);
        }
    }

    #[tokio::test]
    async fn resolve_github_token_errors_clearly_with_nothing_configured() {
        let _guard = ENV_LOCK.lock().await;
        unsafe {
            clear_env(ENV_VARS_UNDER_TEST);
        }
        let err = resolve_github_token("owner", "repo").await.unwrap_err();
        assert!(err.to_string().contains("GITHUB_TOKEN"));
        assert!(err.to_string().contains("GH_APP_CLIENT_ID"));
    }
}
