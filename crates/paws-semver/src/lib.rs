//! Rust port of `gh-reusable`'s `actions/semver` composite action.
//!
//! Parity source (read directly, not re-derived from README): `actions/semver/src/tag.js`
//! (last-tag lookup + prefix inference), `actions/semver/src/increment.js` (label/branch
//! increment precedence), `actions/semver/src/version.js` (new-version construction).
//! See specs/001-paws-core-cli/spec.md FR-003 and FR-011 for the resolved contract this
//! crate exists to satisfy.

use std::future::Future;
use std::pin::Pin;

use anyhow::{Context, Result};

/// A semver increment step. Mirrors `increment.js`'s three valid increment strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Increment {
    Major,
    Minor,
    Patch,
}

impl std::str::FromStr for Increment {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "major" => Ok(Increment::Major),
            "minor" => Ok(Increment::Minor),
            "patch" => Ok(Increment::Patch),
            other => anyhow::bail!("invalid increment type: {other}"),
        }
    }
}

/// Branch-name → increment fallback rules, ported from `increment.js`'s
/// `BRANCH_INCREMENT_RULES`. The original uses regexes anchored on `/` or `-`
/// word boundaries (or start/end of string); token-splitting on `/`/`-` and
/// comparing whole tokens reproduces the same match set without a `regex`
/// dependency (a branch like "hotfixes" must NOT match "fix").
fn resolve_increment_from_branch(branch_name: &str) -> Option<Increment> {
    if branch_name.is_empty() {
        return None;
    }
    let lower = branch_name.to_lowercase();

    // `^release\/.+` is anchored on a literal slash, not a token boundary;
    // checked separately before falling back to hyphen/slash tokenization.
    if lower.starts_with("release/") && lower.len() > "release/".len() {
        return Some(Increment::Minor);
    }

    let tokens: Vec<&str> = lower
        .split(['/', '-'])
        .filter(|s| !s.is_empty())
        .collect();

    const MAJOR_WORDS: &[&str] = &["major", "breaking"];
    const MINOR_WORDS: &[&str] = &["feat", "feature", "minor"];
    const PATCH_WORDS: &[&str] = &[
        "fix", "patch", "hotfix", "bugfix", "chore", "docs", "refactor", "test", "ci", "perf",
    ];

    if tokens.iter().any(|t| MAJOR_WORDS.contains(t)) {
        return Some(Increment::Major);
    }
    if tokens.iter().any(|t| MINOR_WORDS.contains(t)) {
        return Some(Increment::Minor);
    }
    if tokens.iter().any(|t| PATCH_WORDS.contains(t)) {
        return Some(Increment::Patch);
    }
    None
}

fn has_configured_increment_label(labels: &[String], major: &str, minor: &str, patch: &str) -> bool {
    labels
        .iter()
        .any(|l| l == major || l == minor || l == patch)
}

// `increment.js`'s `resolveIncrementFromLabels` has an explicit patch-label
// branch that produces the same result as its final fallback; kept as two
// arms for parity with the original rather than collapsed into one.
#[allow(clippy::if_same_then_else)]
fn resolve_increment_from_labels(labels: &[String], major: &str, minor: &str, patch: &str) -> Increment {
    if labels.iter().any(|l| l == major) {
        Increment::Major
    } else if labels.iter().any(|l| l == minor) {
        Increment::Minor
    } else if labels.iter().any(|l| l == patch) {
        Increment::Patch
    } else {
        Increment::Patch
    }
}

/// Resolved precedence per spec.md FR-011 item 2-3 / `increment.js`'s `detectIncrement`:
/// explicit `--increment` wins outright; otherwise a configured label wins over branch
/// inference; otherwise fall back to `patch`.
pub fn detect_increment(
    explicit_increment: Option<Increment>,
    labels: &[String],
    branch_name: &str,
    major_label: &str,
    minor_label: &str,
    patch_label: &str,
) -> Increment {
    if let Some(increment) = explicit_increment {
        return increment;
    }
    if has_configured_increment_label(labels, major_label, minor_label, patch_label) {
        return resolve_increment_from_labels(labels, major_label, minor_label, patch_label);
    }
    if let Some(increment) = resolve_increment_from_branch(branch_name) {
        return increment;
    }
    Increment::Patch
}

/// Fetches the set of tag names a repository currently has. The real
/// implementation talks to GitHub's GraphQL API (see [`GitHubGraphQlTagSource`]);
/// tests supply a fixed fixture instead so `detect_increment`/`resolve_last_tag`
/// stay testable without live network access (task 27's mockability requirement).
pub trait TagSource: Send + Sync {
    fn tags(&self) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send>>;
}

/// A fixed-list fixture, e.g. `FixtureTagSource(vec!["v1.0.0".into()])`.
pub struct FixtureTagSource(pub Vec<String>);

impl TagSource for FixtureTagSource {
    fn tags(&self) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send>> {
        let tags = self.0.clone();
        Box::pin(async move { Ok(tags) })
    }
}

/// Queries GitHub's GraphQL API for a repository's tags, matching
/// `actions/semver/queries/get_last_tag.gql` (refs under `refs/tags/`, newest
/// commit date first, up to 100).
pub struct GitHubGraphQlTagSource {
    pub owner: String,
    pub repo: String,
    pub token: String,
}

const GET_LAST_TAG_QUERY: &str = r#"query GetLastTag($owner: String!, $repo: String!) {
  repository(owner: $owner, name: $repo) {
    refs(
      refPrefix: "refs/tags/"
      first: 100
      orderBy: { field: TAG_COMMIT_DATE, direction: DESC }
    ) {
      nodes {
        name
      }
    }
  }
}"#;

impl TagSource for GitHubGraphQlTagSource {
    fn tags(&self) -> Pin<Box<dyn Future<Output = Result<Vec<String>>> + Send>> {
        let owner = self.owner.clone();
        let repo = self.repo.clone();
        let token = self.token.clone();
        Box::pin(async move {
            let client = reqwest::Client::new();
            let body = serde_json::json!({
                "query": GET_LAST_TAG_QUERY,
                "variables": { "owner": owner, "repo": repo },
            });
            let response: serde_json::Value = client
                .post("https://api.github.com/graphql")
                .bearer_auth(token)
                .header("User-Agent", "paws-semver")
                .json(&body)
                .send()
                .await
                .context("failed to reach GitHub GraphQL API")?
                .json()
                .await
                .context("failed to parse GitHub GraphQL response")?;

            let nodes = response
                .pointer("/data/repository/refs/nodes")
                .and_then(|v| v.as_array())
                .context("unexpected GraphQL response shape fetching tags")?;

            Ok(nodes
                .iter()
                .filter_map(|node| node.get("name").and_then(|n| n.as_str()))
                .map(String::from)
                .collect())
        })
    }
}

/// Result of last-tag resolution: the tag itself and the prefix that produced it.
/// Mirrors `tag.js`'s `getLastTag` return shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastTag {
    pub tag: String,
    pub prefix: String,
}

/// Resolves the last tag and its prefix per `tag.js`'s `getLastTag`, minus the
/// `GITHUB_REF`-is-a-tag short-circuit and `base` override, which are handled by
/// the caller (see [`compute_new_version`]) before this is invoked.
pub async fn resolve_last_tag(tag_source: &dyn TagSource, prefix: Option<String>) -> Result<LastTag> {
    let tags = tag_source.tags().await?;

    if tags.is_empty() {
        let default_prefix = prefix.unwrap_or_else(|| "v".to_string());
        return Ok(LastTag {
            tag: format!("{default_prefix}0.0.0"),
            prefix: default_prefix,
        });
    }

    // If no prefix was given but every tag starts with "v", infer "v".
    let mut updated_prefix = prefix;
    if updated_prefix.is_none() && tags.iter().all(|t| t.starts_with('v')) {
        updated_prefix = Some("v".to_string());
    }

    let base_prefix = updated_prefix.clone().unwrap_or_default();
    let mut prefix_candidates = vec![base_prefix.clone()];
    if !base_prefix.is_empty() && !base_prefix.ends_with('-') {
        let hyphenated = format!("{base_prefix}-");
        if !prefix_candidates.contains(&hyphenated) {
            prefix_candidates.push(hyphenated);
        }
    }

    let build_candidate_set = |candidate_prefix: &str| -> Vec<(String, semver::Version)> {
        tags.iter()
            .filter_map(|tag| {
                if !candidate_prefix.is_empty() && !tag.starts_with(candidate_prefix) {
                    return None;
                }
                let version_part = &tag[candidate_prefix.len()..];
                semver::Version::parse(version_part)
                    .ok()
                    .map(|v| (tag.clone(), v))
            })
            .collect()
    };

    let mut selected_prefix = base_prefix.clone();
    let mut semver_tags: Vec<(String, semver::Version)> = Vec::new();
    for candidate in &prefix_candidates {
        let candidate_tags = build_candidate_set(candidate);
        if candidate_tags.len() > semver_tags.len() {
            semver_tags = candidate_tags;
            selected_prefix = candidate.clone();
        }
    }

    if semver_tags.is_empty() {
        let default_prefix = updated_prefix.unwrap_or_else(|| "v".to_string());
        return Ok(LastTag {
            tag: format!("{default_prefix}0.0.0"),
            prefix: default_prefix,
        });
    }

    semver_tags.sort_by(|a, b| b.1.cmp(&a.1));
    Ok(LastTag {
        tag: semver_tags[0].0.clone(),
        prefix: selected_prefix,
    })
}

/// Builds the new version string per `version.js`'s `buildNewVersion`.
fn build_new_version(
    last_tag: &str,
    prefix: &str,
    increment: Increment,
    is_pr: bool,
    sha: &str,
) -> Result<String> {
    let version_part = last_tag.strip_prefix(prefix).unwrap_or(last_tag);
    let mut parsed = semver::Version::parse(version_part)
        .with_context(|| format!("invalid semver: {version_part}"))?;

    match increment {
        Increment::Major => {
            parsed.major += 1;
            parsed.minor = 0;
            parsed.patch = 0;
        }
        Increment::Minor => {
            parsed.minor += 1;
            parsed.patch = 0;
        }
        Increment::Patch => parsed.patch += 1,
    }
    parsed.pre = semver::Prerelease::EMPTY;
    parsed.build = semver::BuildMetadata::EMPTY;

    if is_pr {
        let short_sha: String = sha.chars().take(7).collect();
        return Ok(format!(
            "{prefix}{}.{}.{}-pr.{short_sha}",
            parsed.major, parsed.minor, parsed.patch
        ));
    }

    Ok(format!("{prefix}{parsed}"))
}

/// Inputs for a full [`compute_new_version`] call, gathering everything the
/// original action reads from CLI inputs / GitHub Actions context.
#[derive(Debug, Clone, Default)]
pub struct SemverRequest {
    pub prefix: Option<String>,
    pub explicit_increment: Option<Increment>,
    pub major_label: String,
    pub minor_label: String,
    pub patch_label: String,
    pub labels: Vec<String>,
    pub branch_name: String,
    pub sha: String,
    pub is_pr: bool,
    /// Verbatim `GITHUB_REF`, e.g. `refs/tags/v1.2.3` or `refs/heads/main`.
    pub github_ref: Option<String>,
    /// Explicit base version, skipping tag lookup entirely (`tag.js`'s `base` input).
    pub base: Option<String>,
}

impl SemverRequest {
    pub fn new() -> Self {
        Self {
            major_label: "major".to_string(),
            minor_label: "minor".to_string(),
            patch_label: "patch".to_string(),
            ..Default::default()
        }
    }
}

/// Computes the next version for `request`, matching `actions/semver`'s end-to-end
/// behavior (FR-011's full resolved precedence): tag-ref passthrough first, then
/// explicit/label/branch increment resolution, then version construction.
pub async fn compute_new_version(tag_source: &dyn TagSource, request: &SemverRequest) -> Result<String> {
    if let Some(ref_) = &request.github_ref
        && let Some(tag_name) = ref_.strip_prefix("refs/tags/") {
            let prefix = request.prefix.as_deref().unwrap_or("");
            let version_part = tag_name.strip_prefix(prefix).unwrap_or(tag_name);
            // node-semver (used by the original action) accepts an optional
            // leading "v" regardless of the configured prefix; Rust's `semver`
            // crate is strict, so fall back to stripping one before validating.
            semver::Version::parse(version_part)
                .or_else(|_| semver::Version::parse(version_part.strip_prefix('v').unwrap_or(version_part)))
                .with_context(|| format!("tag \"{tag_name}\" is not a valid semantic version"))?;
            return Ok(tag_name.to_string());
        }

    let last_tag = if let Some(base) = &request.base {
        LastTag {
            tag: base.clone(),
            prefix: request.prefix.clone().unwrap_or_default(),
        }
    } else {
        resolve_last_tag(tag_source, request.prefix.clone()).await?
    };
    let increment = detect_increment(
        request.explicit_increment,
        &request.labels,
        &request.branch_name,
        &request.major_label,
        &request.minor_label,
        &request.patch_label,
    );

    build_new_version(&last_tag.tag, &last_tag.prefix, increment, request.is_pr, &request.sha)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_request() -> SemverRequest {
        SemverRequest::new()
    }

    #[tokio::test]
    async fn tagged_repo_with_label_set_increments_major() {
        let tags = FixtureTagSource(vec!["v1.0.0".to_string()]);
        let request = SemverRequest {
            labels: vec!["major".to_string()],
            branch_name: "main".to_string(),
            sha: "abcdef1234567".to_string(),
            ..base_request()
        };

        let version = compute_new_version(&tags, &request).await.unwrap();
        assert_eq!(version, "v2.0.0");
    }

    #[tokio::test]
    async fn explicit_increment_overrides_labels() {
        let tags = FixtureTagSource(vec!["v1.0.0".to_string()]);
        let request = SemverRequest {
            explicit_increment: Some(Increment::Patch),
            labels: vec!["major".to_string()],
            branch_name: "main".to_string(),
            sha: "abcdef1234567".to_string(),
            ..base_request()
        };

        let version = compute_new_version(&tags, &request).await.unwrap();
        assert_eq!(version, "v1.0.1");
    }

    #[tokio::test]
    async fn tagless_repo_defaults_to_prefixed_zero_version() {
        let tags = FixtureTagSource(vec![]);
        let request = SemverRequest {
            branch_name: "main".to_string(),
            sha: "abcdef1234567".to_string(),
            ..base_request()
        };

        let version = compute_new_version(&tags, &request).await.unwrap();
        // No tags -> last tag is "v0.0.0"; no labels/branch match -> patch default.
        assert_eq!(version, "v0.0.1");
    }

    #[tokio::test]
    async fn configured_label_takes_precedence_over_branch_name() {
        let tags = FixtureTagSource(vec!["v1.0.0".to_string()]);
        let request = SemverRequest {
            labels: vec!["patch".to_string()],
            branch_name: "feat/something-big".to_string(),
            sha: "abcdef1234567".to_string(),
            ..base_request()
        };

        // Branch name alone would infer "minor" (feat/...), but a configured
        // patch label on the PR wins per FR-011 item 3.
        let version = compute_new_version(&tags, &request).await.unwrap();
        assert_eq!(version, "v1.0.1");
    }

    #[tokio::test]
    async fn branch_name_infers_increment_when_no_labels_present() {
        let tags = FixtureTagSource(vec!["v1.0.0".to_string()]);
        let request = SemverRequest {
            branch_name: "feat/something-big".to_string(),
            sha: "abcdef1234567".to_string(),
            ..base_request()
        };

        let version = compute_new_version(&tags, &request).await.unwrap();
        assert_eq!(version, "v1.1.0");
    }

    #[tokio::test]
    async fn prefix_is_inferred_from_existing_v_prefixed_tags() {
        let tags = FixtureTagSource(vec!["v1.0.0".to_string(), "v1.1.0".to_string()]);
        let request = SemverRequest {
            branch_name: "main".to_string(),
            sha: "abcdef1234567".to_string(),
            ..base_request()
        };

        let version = compute_new_version(&tags, &request).await.unwrap();
        // Highest tag is v1.1.0; prefix inferred as "v"; default patch increment.
        assert_eq!(version, "v1.1.1");
    }

    #[tokio::test]
    async fn pr_build_produces_prerelease_with_short_sha() {
        let tags = FixtureTagSource(vec!["v1.0.0".to_string()]);
        let request = SemverRequest {
            is_pr: true,
            branch_name: "fix/bug".to_string(),
            sha: "abcdef1234567890".to_string(),
            ..base_request()
        };

        let version = compute_new_version(&tags, &request).await.unwrap();
        assert_eq!(version, "v1.0.1-pr.abcdef1");
    }

    #[tokio::test]
    async fn explicit_base_skips_tag_lookup() {
        let tags = FixtureTagSource(vec!["v1.0.0".to_string()]);
        let request = SemverRequest {
            base: Some("v5.0.0".to_string()),
            prefix: Some("v".to_string()),
            branch_name: "main".to_string(),
            sha: "abcdef1234567".to_string(),
            ..base_request()
        };

        let version = compute_new_version(&tags, &request).await.unwrap();
        assert_eq!(version, "v5.0.1");
    }

    #[tokio::test]
    async fn running_on_a_tag_ref_returns_it_verbatim() {
        let tags = FixtureTagSource(vec!["v1.0.0".to_string()]);
        let request = SemverRequest {
            github_ref: Some("refs/tags/v9.9.9".to_string()),
            ..base_request()
        };

        let version = compute_new_version(&tags, &request).await.unwrap();
        assert_eq!(version, "v9.9.9");
    }
}
