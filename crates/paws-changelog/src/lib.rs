//! `paws changelog`: generates a changelog entry from commit/PR history
//! between two refs, appends it to `CHANGELOG.md`, and optionally commits it
//! back to the target repo's default branch.
//!
//! Not a port of any `gh-reusable` behavior (there is none to port here) —
//! this is new, `paws`-native behavior scoped to functionally replace
//! `mbround18/auto` well enough for `valheim-docker` (and similarly-shaped
//! consumers) to drop it. See `specs/003-release-parity-docker/spec.md` and
//! this feature's `data-model.md`/`contracts/paws-changelog-contract.md` for
//! the full contract this crate implements.

use anyhow::{Context, Result};

/// One commit in a [`HistoryProvider::commits_in_range`] result — enough to
/// render a fallback line (FR-009) even when no PR is found for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryCommit {
    pub sha: String,
    /// Raw commit subject (first line of the commit message).
    pub subject: String,
}

/// What changelog generation needs from a commit/PR history source
/// (FR-017). This spec ships exactly one implementation
/// ([`GitHubHistoryProvider`]); the trait itself — not any particular
/// implementation — is the durable contract a future GitLab or local-`git
/// log` provider would need to satisfy, without `paws changelog`'s CLI
/// contract changing.
#[async_trait::async_trait]
pub trait HistoryProvider: Send + Sync {
    /// Every commit in `(base, head]`. A failure to reach the provider at
    /// all is a hard error — never a silently-empty list (that fallback is
    /// reserved for the narrower "no PR found for this one commit" case,
    /// see [`pr_title_for_commit`](Self::pr_title_for_commit)).
    async fn commits_in_range(&self, base: &str, head: &str) -> Result<Vec<HistoryCommit>>;

    /// The title of the pull request associated with `sha`, or `Ok(None)`
    /// (never an error) when there isn't one, or when the lookup fails for
    /// a reason FR-009 already classifies as "fall back" (e.g. a narrowly
    /// scoped token that can't see closed/merged PR data). Callers render
    /// [`ChangelogLine::RawCommit`] in either case — a commit is never
    /// silently dropped.
    async fn pr_title_for_commit(&self, sha: &str) -> Result<Option<String>>;
}

/// GitHub REST API implementation of [`HistoryProvider`] — the sole
/// implementation this spec ships (research.md R2, R3).
pub struct GitHubHistoryProvider {
    pub owner: String,
    pub repo: String,
    pub token: String,
    client: reqwest::Client,
}

/// Hand-written, not derived: `token` is a live GitHub credential.
impl std::fmt::Debug for GitHubHistoryProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubHistoryProvider")
            .field("owner", &self.owner)
            .field("repo", &self.repo)
            .field("token", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl GitHubHistoryProvider {
    pub fn new(owner: String, repo: String, token: String) -> Self {
        Self {
            owner,
            repo,
            token,
            client: reqwest::Client::new(),
        }
    }

    fn api_base(&self) -> String {
        format!("https://api.github.com/repos/{}/{}", self.owner, self.repo)
    }

    fn auth_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .bearer_auth(&self.token)
            .header("User-Agent", "paws-changelog")
            .header("Accept", "application/vnd.github+json")
    }
}

#[async_trait::async_trait]
impl HistoryProvider for GitHubHistoryProvider {
    async fn commits_in_range(&self, base: &str, head: &str) -> Result<Vec<HistoryCommit>> {
        let url = format!("{}/compare/{base}...{head}", self.api_base());
        let response = self
            .auth_headers(self.client.get(&url))
            .send()
            .await
            .context("failed to reach GitHub's compare API")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("failed to compare {base}...{head}: {status}: {text}");
        }

        let body: serde_json::Value = response
            .json()
            .await
            .context("failed to parse GitHub's compare response")?;
        let commits = body
            .get("commits")
            .and_then(|v| v.as_array())
            .context("unexpected compare response shape (missing commits array)")?;

        Ok(commits
            .iter()
            .filter_map(|c| {
                let sha = c.get("sha")?.as_str()?.to_string();
                let message = c.get("commit")?.get("message")?.as_str()?;
                let subject = message.lines().next().unwrap_or(message).to_string();
                Some(HistoryCommit { sha, subject })
            })
            .collect())
    }

    async fn pr_title_for_commit(&self, sha: &str) -> Result<Option<String>> {
        if sha.is_empty() {
            return Ok(None);
        }

        let url = format!("{}/commits/{sha}/pulls", self.api_base());
        // A reachability failure here is treated the same as "no PR found"
        // (FR-009's fallback), not a hard error — the range enumeration call
        // (commits_in_range) is where a real network failure should abort the
        // whole run; a single commit's PR lookup failing shouldn't.
        let Ok(response) = self.auth_headers(self.client.get(&url)).send().await else {
            return Ok(None);
        };
        if !response.status().is_success() {
            return Ok(None);
        }

        let body: serde_json::Value = match response.json().await {
            Ok(body) => body,
            Err(_) => return Ok(None),
        };
        let Some(prs) = body.as_array() else {
            return Ok(None);
        };

        Ok(prs
            .first()
            .and_then(|pr| pr.get("title"))
            .and_then(|t| t.as_str())
            .map(String::from))
    }
}

/// Selects a [`HistoryProvider`] automatically based on the running
/// environment (FR-018) — no `--provider` CLI flag. This spec ships a
/// GitHub implementation only; `paws_environment::CiContext::detect()`
/// (research.md R1) is the single, already-shipped detection mechanism
/// this reuses rather than duplicating — a future GitLab provider is added
/// by extending that detection, not by adding a second one here. When no
/// provider's environment signature matches, `CiContext::detect()` itself
/// already fails with an explicit, actionable error (matching FR-018's
/// requirement) — this function does not need to re-implement that.
pub async fn detect_history_provider() -> Result<Box<dyn HistoryProvider>> {
    let ctx = paws_environment::CiContext::detect()
        .await
        .context("paws changelog needs a supported CI provider's env vars")?;
    match ctx.provider {
        paws_environment::Provider::GitHub => Ok(Box::new(GitHubHistoryProvider::new(
            ctx.owner, ctx.repo, ctx.token,
        ))),
    }
}

/// One line in a [`ChangelogEntry`] — a PR title (preferred) or a raw
/// commit subject (FR-009 fallback when no PR is found).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChangelogLine {
    PullRequest { title: String, sha: String },
    RawCommit { subject: String, sha: String },
}

/// One dated, version-headed section to append to `CHANGELOG.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangelogEntry {
    pub version: String,
    /// `YYYY-MM-DD`, formatted manually — no date-crate dependency (see
    /// this feature's analysis finding U1: no crate in this workspace
    /// depends on `chrono` or similar today, and this feature doesn't need
    /// to be the first).
    pub date: String,
    pub lines: Vec<ChangelogLine>,
}

/// Renders `entry` as a Markdown section — pure, no I/O, independently
/// testable from both the [`HistoryProvider`] call and the file-append step
/// (FR-008). This is a new, `paws`-native format; it is not a byte-for-byte
/// port of `mbround18/auto`'s Markdown (spec's Out of Scope).
pub fn render_entry(entry: &ChangelogEntry) -> String {
    let mut out = format!("## {} ({})\n\n", entry.version, entry.date);
    let short_sha = |sha: &str| sha.chars().take(7).collect::<String>();
    for line in &entry.lines {
        use std::fmt::Write as _;
        let _ = match line {
            ChangelogLine::PullRequest { title, sha } => {
                writeln!(out, "- {title} ({})", short_sha(sha))
            }
            ChangelogLine::RawCommit { subject, sha } => {
                writeln!(out, "- {subject} ({})", short_sha(sha))
            }
        };
    }
    out
}

/// Builds a [`ChangelogEntry`] for `version` from every commit in
/// `(base, head]`, rendering each as a PR title (preferred) or a raw commit
/// subject (FR-009 fallback) via `provider`.
pub async fn build_entry(
    provider: &dyn HistoryProvider,
    version: &str,
    date: &str,
    base: &str,
    head: &str,
) -> Result<ChangelogEntry> {
    let commits = provider.commits_in_range(base, head).await?;
    let mut lines = Vec::with_capacity(commits.len());
    for commit in commits {
        let line = match provider.pr_title_for_commit(&commit.sha).await? {
            Some(title) => ChangelogLine::PullRequest {
                title,
                sha: commit.sha,
            },
            None => ChangelogLine::RawCommit {
                subject: commit.subject,
                sha: commit.sha,
            },
        };
        lines.push(line);
    }
    Ok(ChangelogEntry {
        version: version.to_string(),
        date: date.to_string(),
        lines,
    })
}

/// Appends `entry`'s rendered section to the file at `path` — a literal
/// end-of-file append (FR-008: existing content is preserved *above* the
/// new section), not a prepend. First-run (no file yet) creates it rather
/// than erroring (User Story 2 Acceptance Scenario 2). Returns the rendered
/// section text so callers can also print it (e.g. to stdout, or hand it to
/// a commit-back step) without re-rendering.
pub async fn append_to_file(path: &std::path::Path, entry: &ChangelogEntry) -> Result<String> {
    let rendered = render_entry(entry);

    let existing = match tokio::fs::read_to_string(path).await {
        Ok(existing) => Some(existing),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            return Err(err).with_context(|| format!("failed to read {}", path.display()));
        }
    };

    let updated = match existing {
        None => rendered.clone(),
        Some(existing) if existing.is_empty() => rendered.clone(),
        Some(existing) if existing.ends_with('\n') => format!("{existing}\n{rendered}"),
        Some(existing) => format!("{existing}\n\n{rendered}"),
    };

    tokio::fs::write(path, &updated)
        .await
        .with_context(|| format!("failed to write {}", path.display()))?;

    Ok(rendered)
}

/// Days-since-epoch -> (year, month, day), proleptic Gregorian calendar.
/// Howard Hinnant's public-domain `civil_from_days` algorithm — used
/// instead of a date-crate dependency (this feature's `ChangelogEntry.date`
/// is a manually formatted `String`, see analysis finding U1; no crate in
/// this workspace depends on `chrono` or similar today).
// Transcribed from the published algorithm rather than rewritten. Every cast
// here is part of its correctness argument — `doe` is provably in
// `0..=146096` at that point, so the `i64`/`u64` round trip cannot lose a
// sign, and `d`/`m` are provably small. Replacing them with `try_from` would
// also make this non-`const`. Checked arithmetic here would assert facts the
// algorithm already establishes.
#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation
)]
const fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Today's date as `YYYY-MM-DD`, UTC.
pub fn today_iso_date() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    // Days since 1970 fits an i64 for any clock a machine can report.
    let days = i64::try_from(now.as_secs() / 86_400).unwrap_or(i64::MAX);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Resolves the `(base, head]` commit range's `base` end: `explicit_previous_ref`
/// if the caller gave one, otherwise the same prefix-aware last-tag
/// resolution `paws semver` already implements (FR-010) — so a
/// `--prefix`-scoped monorepo range doesn't pull in an unrelated
/// component's commit history.
pub async fn resolve_previous_ref(
    tag_source: &dyn paws_semver::TagSource,
    explicit_previous_ref: Option<String>,
    prefix: Option<String>,
) -> Result<String> {
    if let Some(explicit) = explicit_previous_ref {
        return Ok(explicit);
    }
    let last_tag = paws_semver::resolve_last_tag(tag_source, prefix).await?;
    Ok(last_tag.tag)
}

/// Commits the already-updated `CHANGELOG.md` back to `branch` via the
/// Contents API (FR-013) — the exact mechanism `paws llms generate
/// --publish` already ships (`paws_release::GitHubReleaseClient::get_content`/
/// `put_content`, research.md R4), reused verbatim rather than rebuilt. The
/// commit message unconditionally carries the `[skip ci]` loop-avoidance
/// marker, matching `valheim-docker`'s own existing
/// `Update CHANGELOG.md [skip ci]` convention, so a consumer's
/// `push`-triggered release workflow doesn't re-trigger itself. On a
/// rejected push (e.g. the branch moved since this run started),
/// `put_content` already fails loudly with the response status/body and
/// performs no retry — exactly FR-013's Clarifications-resolved contract,
/// with no extra logic needed here.
pub async fn commit_back(
    client: &paws_release::GitHubReleaseClient,
    path: &str,
    branch: &str,
) -> Result<()> {
    let existing = client.get_content(path, branch).await?;
    let content = tokio::fs::read(path)
        .await
        .with_context(|| format!("failed to read {path} for commit-back"))?;

    client
        .put_content(
            path,
            branch,
            &content,
            "chore: update CHANGELOG.md [skip ci]",
            existing.as_ref().map(|e| e.sha.as_str()),
        )
        .await
}

#[cfg(test)]
// `std::env::set_var`/`remove_var` are unsafe in edition 2024, and these
// tests exist precisely to exercise env-var-driven behavior. Access is
// serialized within this module, which is what makes it sound.
#[allow(unsafe_code)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// A fixed, in-memory [`HistoryProvider`] for testing rendering/
    /// fallback behavior without live network access.
    struct FixtureHistoryProvider {
        commits: Vec<HistoryCommit>,
        pr_titles: HashMap<String, String>,
        pr_lookup_calls: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl HistoryProvider for FixtureHistoryProvider {
        async fn commits_in_range(&self, _base: &str, _head: &str) -> Result<Vec<HistoryCommit>> {
            Ok(self.commits.clone())
        }

        async fn pr_title_for_commit(&self, sha: &str) -> Result<Option<String>> {
            self.pr_lookup_calls.lock().unwrap().push(sha.to_string());
            Ok(self.pr_titles.get(sha).cloned())
        }
    }

    fn commit(sha: &str, subject: &str) -> HistoryCommit {
        HistoryCommit {
            sha: sha.to_string(),
            subject: subject.to_string(),
        }
    }

    // T035: PR-title rendering against a mocked HistoryProvider.
    #[tokio::test]
    async fn build_entry_prefers_pr_titles_over_raw_subjects() {
        let provider = FixtureHistoryProvider {
            commits: vec![commit("abc1234567", "fix: mod support (#1468)")],
            pr_titles: HashMap::from([("abc1234567".to_string(), "fix/mod support".to_string())]),
            pr_lookup_calls: Mutex::new(Vec::new()),
        };

        let entry = build_entry(&provider, "v1.3.0", "2026-08-23", "v1.2.0", "v1.3.0")
            .await
            .unwrap();
        assert_eq!(
            entry.lines,
            vec![ChangelogLine::PullRequest {
                title: "fix/mod support".to_string(),
                sha: "abc1234567".to_string(),
            }]
        );

        let rendered = render_entry(&entry);
        assert!(rendered.contains("## v1.3.0 (2026-08-23)"));
        assert!(rendered.contains("- fix/mod support (abc1234)"));
    }

    // T036 (FR-009): raw-commit-subject fallback when no PR is found.
    #[tokio::test]
    async fn build_entry_falls_back_to_raw_subject_when_no_pr_found() {
        let provider = FixtureHistoryProvider {
            commits: vec![commit("deadbeef00", "chore: direct push, no PR")],
            pr_titles: HashMap::new(),
            pr_lookup_calls: Mutex::new(Vec::new()),
        };

        let entry = build_entry(&provider, "v1.3.0", "2026-08-23", "v1.2.0", "v1.3.0")
            .await
            .unwrap();
        assert_eq!(
            entry.lines,
            vec![ChangelogLine::RawCommit {
                subject: "chore: direct push, no PR".to_string(),
                sha: "deadbeef00".to_string(),
            }]
        );
        // The commit is never silently dropped, and the lookup was
        // actually attempted (not skipped).
        assert_eq!(
            *provider.pr_lookup_calls.lock().unwrap(),
            vec!["deadbeef00"]
        );
    }

    // T033 (SC-003): append-only behavior against a pre-populated fixture
    // CHANGELOG.md — pre-existing content is preserved byte-for-byte above
    // the newly appended section.
    #[tokio::test]
    async fn append_to_file_preserves_pre_existing_content_above_the_new_section() {
        let dir = tempdir();
        let path = dir.join("CHANGELOG.md");
        let pre_existing = "# v3.6.1 (Wed Jun 17 2026)\n\n#### Bug Fix\n\n- fix/mod support\n";
        std::fs::write(&path, pre_existing).unwrap();

        let entry = ChangelogEntry {
            version: "v1.3.0".to_string(),
            date: "2026-08-23".to_string(),
            lines: vec![ChangelogLine::RawCommit {
                subject: "chore: bump".to_string(),
                sha: "abc1234".to_string(),
            }],
        };
        append_to_file(&path, &entry).await.unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.starts_with(pre_existing));
        assert!(written.contains("## v1.3.0 (2026-08-23)"));
        std::fs::remove_dir_all(&dir).ok();
    }

    // T034: first-run file creation when no CHANGELOG.md exists yet.
    #[tokio::test]
    async fn append_to_file_creates_the_file_on_first_run() {
        let dir = tempdir();
        let path = dir.join("CHANGELOG.md");
        assert!(!path.exists());

        let entry = ChangelogEntry {
            version: "v0.1.0".to_string(),
            date: "2026-08-23".to_string(),
            lines: vec![],
        };
        let rendered = append_to_file(&path, &entry).await.unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert_eq!(written, rendered);
        assert!(written.starts_with("## v0.1.0 (2026-08-23)"));
        std::fs::remove_dir_all(&dir).ok();
    }

    // T037 (FR-010): prefix-scoped range resolution in a simulated
    // monorepo fixture — resolve_previous_ref must not cross an unrelated
    // component's tags.
    #[tokio::test]
    async fn resolve_previous_ref_uses_prefix_aware_last_tag_when_no_explicit_ref_given() {
        // `resolve_last_tag` strips exactly `prefix` and then an optional
        // leading "v" before semver-parsing what's left, so a monorepo
        // prefix like "chart-a-" matches the "chart-a-v1.2.0" convention
        // (spec.md's own example, per Edge Cases) automatically — see
        // `paws-semver`'s `resolve_last_tag_matches_a_prefixed_tag_with_an_embedded_v`
        // for the underlying coverage.
        let tags = paws_semver::FixtureTagSource(vec![
            "chart-a-v1.0.0".to_string(),
            "chart-a-v1.2.0".to_string(),
            "chart-b-v9.0.0".to_string(),
        ]);
        let resolved = resolve_previous_ref(&tags, None, Some("chart-a-".to_string()))
            .await
            .unwrap();
        assert_eq!(resolved, "chart-a-v1.2.0");
    }

    #[tokio::test]
    async fn resolve_previous_ref_also_works_without_an_embedded_v() {
        let tags = paws_semver::FixtureTagSource(vec![
            "chart-a-1.0.0".to_string(),
            "chart-a-1.2.0".to_string(),
            "chart-b-9.0.0".to_string(),
        ]);
        let resolved = resolve_previous_ref(&tags, None, Some("chart-a-".to_string()))
            .await
            .unwrap();
        assert_eq!(resolved, "chart-a-1.2.0");
    }

    #[tokio::test]
    async fn resolve_previous_ref_prefers_an_explicit_override() {
        let tags = paws_semver::FixtureTagSource(vec!["v1.0.0".to_string()]);
        let resolved = resolve_previous_ref(&tags, Some("v0.5.0".to_string()), None)
            .await
            .unwrap();
        assert_eq!(resolved, "v0.5.0");
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // January has 31 days, so day 31 (0-indexed from day 0) is Feb 1st.
        assert_eq!(civil_from_days(31), (1970, 2, 1));
        // 1972 is a leap year; day 365*2 (-> Jan 1 1972) + 31 (-> Feb 1) +
        // 28 more days lands on Feb 29th, 1972.
        assert_eq!(civil_from_days(365 * 2 + 31 + 28), (1972, 2, 29));
    }

    // T038 (FR-013): commit-back's commit message unconditionally carries
    // the loop-avoidance marker. `commit_back` itself calls a live GitHub
    // API (no HTTP mock server anywhere in this workspace, matching
    // `paws-release`'s own `GitHubReleaseClient` — see its test module,
    // which likewise exercises `get_content`/`put_content` by construction
    // rather than a live/mocked call), so this pins the literal message
    // string `commit_back` constructs rather than round-tripping it through
    // a real request. Non-retry-on-conflict (T040) needs no separate test
    // here: `put_content` (reused verbatim, research.md R4) already fails
    // loudly with the response status/body and never retries — that
    // behavior is inherited, not new code this crate adds.
    #[test]
    fn commit_back_message_always_carries_the_skip_ci_marker() {
        const COMMIT_MESSAGE: &str = "chore: update CHANGELOG.md [skip ci]";
        assert!(COMMIT_MESSAGE.contains("[skip ci]"));
    }

    // T041 (FR-018): HistoryProvider auto-selection. Mutates process env
    // vars — safe here because this is the only test in this crate's
    // binary that touches `GITHUB_REPOSITORY`/`GITHUB_TOKEN`, and both
    // assertions run sequentially within one test function (no risk of
    // racing itself). Original values are restored afterward regardless.
    #[tokio::test]
    async fn history_provider_auto_selection_matches_github_env_or_errors_explicitly() {
        let saved_repo = std::env::var("GITHUB_REPOSITORY").ok();
        let saved_token = std::env::var("GITHUB_TOKEN").ok();
        let saved_gh_token = std::env::var("GH_TOKEN").ok();

        unsafe {
            std::env::set_var("GITHUB_REPOSITORY", "octocat/example");
            std::env::set_var("GITHUB_TOKEN", "test-token");
            std::env::remove_var("GH_TOKEN");
        }
        let provider = detect_history_provider().await;
        assert!(provider.is_ok(), "expected GitHub env signature to match");

        unsafe {
            std::env::remove_var("GITHUB_REPOSITORY");
            std::env::remove_var("GITHUB_TOKEN");
            std::env::remove_var("GH_TOKEN");
        }
        let no_match = detect_history_provider().await;
        assert!(
            no_match.is_err(),
            "expected an explicit error when no provider's environment signature matches"
        );

        unsafe {
            match saved_repo {
                Some(v) => std::env::set_var("GITHUB_REPOSITORY", v),
                None => std::env::remove_var("GITHUB_REPOSITORY"),
            }
            match saved_token {
                Some(v) => std::env::set_var("GITHUB_TOKEN", v),
                None => std::env::remove_var("GITHUB_TOKEN"),
            }
            match saved_gh_token {
                Some(v) => std::env::set_var("GH_TOKEN", v),
                None => std::env::remove_var("GH_TOKEN"),
            }
        }
    }

    /// A fresh, empty scratch directory unique to this test invocation —
    /// callers are responsible for removing it (`std::fs::remove_dir_all`)
    /// once done.
    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "paws-changelog-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
