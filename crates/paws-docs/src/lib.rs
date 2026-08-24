//! Rust port of `gh-reusable`'s `rustDocsBuild`/`rust-docs-publish.yaml`: run `cargo doc`
//! against a workspace and produce a stable, idempotent output path. See
//! specs/001-paws-core-cli/spec.md User Story 6 for the contract this crate exists to satisfy.
//!
//! Unlike `paws-audit`/`paws-docker`, there's no separate TS source to port parity against —
//! `rustDocsBuild` itself is a thin wrapper around `cargo doc`, so this crate is that same
//! thin wrapper, not a reimplementation of anything `cargo` already does well.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::process::Command;

/// Runs `cargo doc --workspace --no-deps` against `workspace_root` and returns the
/// produced docs directory. `cargo doc` is itself incremental/idempotent — re-running
/// this against an unchanged workspace does not rebuild from scratch and never fails
/// just for having been run before.
pub async fn build_docs(workspace_root: &Path) -> Result<PathBuf> {
    let status = Command::new("cargo")
        .args(["doc", "--workspace", "--no-deps"])
        .current_dir(workspace_root)
        .status()
        .await
        .context("failed to spawn `cargo doc` — is `cargo` on PATH?")?;

    if !status.success() {
        anyhow::bail!("`cargo doc --workspace --no-deps` failed (exit status: {status})");
    }

    Ok(workspace_root.join("target").join("doc"))
}

/// specs/005-close-remaining-cli: which destination(s) `paws docs
/// --provider` publishes the built `target/doc` tree to. Parsed from
/// `--provider`'s comma-delimited values in `paws-cli-core::DocsArgs`
/// (each value independently rejected with an actionable error before any
/// work starts if it isn't one of these three — contracts/paws-docs-publish-contract.md §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishTarget {
    GitHubPages,
    /// FR-004a: recognized, but not implemented yet — fails immediately,
    /// no build/publish attempt.
    CloudflarePages,
    /// FR-004a: recognized, but not implemented yet — fails immediately,
    /// no build/publish attempt.
    S3,
}

impl PublishTarget {
    pub fn as_str(&self) -> &'static str {
        match self {
            PublishTarget::GitHubPages => "github-pages",
            PublishTarget::CloudflarePages => "cloudflare-pages",
            PublishTarget::S3 => "s3",
        }
    }
}

impl std::str::FromStr for PublishTarget {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "github-pages" => Ok(PublishTarget::GitHubPages),
            "cloudflare-pages" => Ok(PublishTarget::CloudflarePages),
            "s3" => Ok(PublishTarget::S3),
            other => anyhow::bail!(
                "invalid --provider value {other:?}; expected one of: github-pages, \
                 cloudflare-pages, s3"
            ),
        }
    }
}

/// FR-004a's fixed error for a recognized-but-unimplemented `PublishTarget`
/// — same wording for every such provider so a caller/test can match on it
/// without caring which specific provider triggered it.
pub fn not_implemented_error(target: PublishTarget) -> anyhow::Error {
    anyhow::anyhow!(
        "--provider {} is not implemented yet — see docs/ROADMAP.md",
        target.as_str()
    )
}

/// Which GitHub Pages publish mechanism a `github-pages` publish attempt
/// uses — resolved once per attempt from the repo's live Pages
/// configuration ([`GitHubReleaseClient::get_pages_config`]), never
/// user-configurable (FR-003; data-model.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitHubPagesMechanism {
    /// `build_type == "legacy"`, or Pages isn't configured yet at all
    /// (404) — the Git Trees bulk-commit path, works from anywhere with a
    /// suitably-scoped token.
    GitTrees,
    /// `build_type == "workflow"` — the Pages deployment/artifact API,
    /// which only works from inside a real GitHub Actions job
    /// (research.md R5).
    PagesDeployment,
}

fn select_pages_mechanism(config: Option<&paws_release::PagesConfig>) -> GitHubPagesMechanism {
    match config.map(|c| c.build_type.as_str()) {
        Some("workflow") => GitHubPagesMechanism::PagesDeployment,
        _ => GitHubPagesMechanism::GitTrees,
    }
}

/// The env vars GitHub Actions' own artifact-upload mechanism requires —
/// the [`GitHubPagesMechanism::PagesDeployment`] path is unusable without
/// them (research.md R5). Returns the *missing* ones, named explicitly, so
/// the caller can fail with an actionable error rather than a confusing
/// failure partway through a doomed deployment call.
fn missing_actions_runtime_env_vars() -> Vec<&'static str> {
    ["ACTIONS_RUNTIME_TOKEN", "ACTIONS_RESULTS_URL"]
        .into_iter()
        .filter(|var| std::env::var(var).is_err())
        .collect()
}

/// Recursively lists every regular file under `root`, returning each as
/// `(repo-relative "/"-separated path, absolute host path)`, sorted by
/// path for deterministic ordering (stable across runs, so a re-publish of
/// unchanged content produces the exact same manifest digest — see
/// [`manifest_digest`]).
fn collect_tree_files(root: &Path) -> Result<Vec<(String, PathBuf)>> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<()> {
        for entry in std::fs::read_dir(dir)
            .with_context(|| format!("failed to read directory {}", dir.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out)?;
            } else {
                let relative = path
                    .strip_prefix(root)
                    .with_context(|| format!("{} is not under {}", path.display(), root.display()))?
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/");
                out.push((relative, path));
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

/// A deterministic digest of a whole file set's path+content, used as the
/// idempotency check ahead of [`publish_github_pages`]'s real
/// `publish_tree` call — mirrors `paws-cli-core::should_publish`'s
/// single-file "identical bytes -> skip" bar, generalized to a whole tree
/// (contracts/paws-docs-publish-contract.md §5).
fn manifest_digest(files: &[(String, Vec<u8>)]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    for (path, content) in files {
        path.hash(&mut hasher);
        content.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

/// The path the current manifest digest is stashed at on the publish
/// branch, alongside the published tree — read back on the next publish
/// attempt to decide whether there's anything new to commit at all.
const MANIFEST_PATH: &str = ".paws-docs-manifest";

/// Publishes `docs_dir`'s full file tree to `branch` via the `github-pages`
/// provider: resolves the publish mechanism from the repo's live Pages
/// configuration (research.md R4), skips as a safe no-op if the content
/// hasn't changed since the last publish (contract §5), and otherwise
/// blob-creates every file and commits the whole tree in one
/// `publish_tree` call (contract §3 — never a per-file loop, FR-003).
pub async fn publish_github_pages(
    client: &paws_release::GitHubReleaseClient,
    branch: &str,
    docs_dir: &Path,
) -> Result<()> {
    let pages_config = client.get_pages_config().await?;
    let mechanism = select_pages_mechanism(pages_config.as_ref());

    if mechanism == GitHubPagesMechanism::PagesDeployment {
        let missing = missing_actions_runtime_env_vars();
        if !missing.is_empty() {
            anyhow::bail!(
                "paws docs --provider github-pages: this repository's Pages site uses the \
                 \"workflow\" build type, which needs a real GitHub Actions job (missing: {}) \
                 — run this inside a GitHub Actions workflow, or switch the repo's Pages source \
                 to a branch (legacy) in Settings > Pages",
                missing.join(", ")
            );
        }
        // The Pages-deployment/artifact-upload API itself is a materially
        // different, Actions-runtime-only call sequence (research.md R5) —
        // out of scope for this pass beyond detecting and gating on it;
        // tracked in docs/ROADMAP.md. The gate above is what T034/T036
        // actually need: fail explicitly rather than attempt a doomed call.
        anyhow::bail!(
            "paws docs --provider github-pages: this repository's Pages site uses the \
             \"workflow\" build type, whose deployment API isn't implemented yet — see \
             docs/ROADMAP.md; switch the repo's Pages source to a branch (legacy) in \
             Settings > Pages to publish via paws today"
        );
    }

    let files = collect_tree_files(docs_dir)?;
    let mut files_with_content = Vec::with_capacity(files.len());
    for (relative_path, absolute_path) in &files {
        let content = tokio::fs::read(absolute_path)
            .await
            .with_context(|| format!("failed to read {}", absolute_path.display()))?;
        files_with_content.push((relative_path.clone(), content));
    }
    let digest = manifest_digest(&files_with_content);

    let existing_manifest = client.get_content(MANIFEST_PATH, branch).await?;
    if existing_manifest
        .as_ref()
        .map(|m| String::from_utf8_lossy(&m.content).trim() == digest)
        .unwrap_or(false)
    {
        println!("docs: github-pages already up to date ({branch}), skipping publish");
        return Ok(());
    }

    let mut blobs = Vec::with_capacity(files_with_content.len() + 1);
    for (relative_path, content) in &files_with_content {
        let sha = client.create_blob(content).await?;
        blobs.push((relative_path.clone(), sha));
    }
    let manifest_blob_sha = client.create_blob(digest.as_bytes()).await?;
    blobs.push((MANIFEST_PATH.to_string(), manifest_blob_sha));

    client
        .publish_tree(
            branch,
            &blobs,
            "docs: publish via paws docs --provider github-pages",
        )
        .await?;
    println!(
        "docs: published {} file(s) to {branch} via github-pages",
        files_with_content.len()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_root() -> PathBuf {
        // crates/paws-docs -> repo root
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[tokio::test]
    async fn building_docs_produces_a_stable_output_path() {
        let docs_dir = build_docs(&repo_root()).await.unwrap();
        assert!(docs_dir.ends_with("target/doc"));
        assert!(
            docs_dir.is_dir(),
            "expected {docs_dir:?} to exist after a successful build"
        );
    }

    #[tokio::test]
    async fn rerunning_against_an_unchanged_workspace_is_still_successful() {
        let workspace = repo_root();
        build_docs(&workspace).await.unwrap();
        // Re-running is the idempotency contract itself: no error, no required cleanup.
        let docs_dir = build_docs(&workspace).await.unwrap();
        assert!(docs_dir.is_dir());
    }

    #[test]
    fn publish_target_parses_and_rejects_correctly() {
        assert_eq!(
            "github-pages".parse::<PublishTarget>().unwrap(),
            PublishTarget::GitHubPages
        );
        assert_eq!(
            "cloudflare-pages".parse::<PublishTarget>().unwrap(),
            PublishTarget::CloudflarePages
        );
        assert_eq!("s3".parse::<PublishTarget>().unwrap(), PublishTarget::S3);
        assert!("azure-static-web-apps".parse::<PublishTarget>().is_err());
    }

    #[test]
    fn select_pages_mechanism_prefers_git_trees_for_legacy_or_unconfigured() {
        assert_eq!(select_pages_mechanism(None), GitHubPagesMechanism::GitTrees);
        assert_eq!(
            select_pages_mechanism(Some(&paws_release::PagesConfig {
                build_type: "legacy".to_string(),
            })),
            GitHubPagesMechanism::GitTrees
        );
        assert_eq!(
            select_pages_mechanism(Some(&paws_release::PagesConfig {
                build_type: "workflow".to_string(),
            })),
            GitHubPagesMechanism::PagesDeployment
        );
    }

    #[test]
    fn collect_tree_files_walks_recursively_and_sorts_by_path() {
        let dir = std::env::temp_dir().join(format!("paws-docs-test-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("b.html"), "b").unwrap();
        std::fs::write(dir.join("sub/a.html"), "a").unwrap();

        let files = collect_tree_files(&dir).unwrap();
        let paths: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, vec!["b.html", "sub/a.html"]);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Serves one canned JSON response per entry in `responses`, in order,
    /// on freshly-accepted connections — mirrors `paws-release`'s own
    /// fixture-server test helper (same shape, duplicated rather than
    /// shared since it's a handful of lines and the two crates' test
    /// modules aren't otherwise coupled).
    async fn serve_fixture_responses(
        listener: tokio::net::TcpListener,
        responses: Vec<(u16, serde_json::Value)>,
    ) -> Vec<String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut requests = Vec::new();
        for (status, body) in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let n = socket.read(&mut buf).await.unwrap();
            requests.push(String::from_utf8_lossy(&buf[..n]).into_owned());

            let payload = body.to_string();
            let status_line = match status {
                200 => "200 OK",
                404 => "404 Not Found",
                other => panic!("unsupported fixture status {other}"),
            };
            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                payload.len(),
                payload
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.ok();
        }
        requests
    }

    fn fixture_docs_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("paws-docs-fixture-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), "<html>hi</html>").unwrap();
        dir
    }

    #[tokio::test]
    async fn publish_github_pages_workflow_without_env_vars_fails_before_any_publish_call() {
        // T036: `build_type: "workflow"` with no Actions-runtime env vars
        // present must fail with the specific named-vars error and never
        // attempt create_blob/publish_tree — the fixture server below only
        // ever answers the one `get_pages_config` request; a second
        // request of any kind would hang waiting for a connection that
        // never comes, which `tokio::time::timeout` below turns into a
        // clear test failure instead of an indefinite hang.
        unsafe {
            std::env::remove_var("ACTIONS_RUNTIME_TOKEN");
            std::env::remove_var("ACTIONS_RESULTS_URL");
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let responses = vec![(200, serde_json::json!({ "build_type": "workflow" }))];
        let server = tokio::spawn(serve_fixture_responses(listener, responses));

        let client = paws_release::GitHubReleaseClient::new(
            "octo".to_string(),
            "repo".to_string(),
            "t".to_string(),
        )
        .with_base_url_for_tests(format!("http://{addr}"));

        let docs_dir = fixture_docs_dir("workflow-gate");
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            publish_github_pages(&client, "main", &docs_dir),
        )
        .await
        .expect("publish_github_pages should fail fast, not hang");

        let err = result.unwrap_err().to_string();
        assert!(err.contains("ACTIONS_RUNTIME_TOKEN"), "error was: {err}");
        assert!(err.contains("ACTIONS_RESULTS_URL"), "error was: {err}");

        server.await.unwrap();
        std::fs::remove_dir_all(&docs_dir).ok();
    }

    #[tokio::test]
    async fn publish_github_pages_skips_republish_when_manifest_digest_is_unchanged() {
        // T038: a manifest digest that already matches means "up to date" —
        // publish_github_pages must return Ok without ever calling
        // create_blob/publish_tree. The fixture below only answers
        // get_pages_config (404 -> Git Trees) and get_content (the
        // manifest lookup) — any further request would hang, and the
        // timeout below turns that into a clear failure instead.
        let dir = fixture_docs_dir("idempotent");
        let files_with_content = vec![(
            "index.html".to_string(),
            std::fs::read(dir.join("index.html")).unwrap(),
        )];
        let digest = manifest_digest(&files_with_content);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let encoded = {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(digest.as_bytes())
        };
        let responses = vec![
            (404, serde_json::Value::Null), // get_pages_config: not configured -> Git Trees
            (
                200,
                serde_json::json!({ "sha": "manifest-sha", "content": encoded }),
            ), // get_content(MANIFEST_PATH)
        ];
        let server = tokio::spawn(serve_fixture_responses(listener, responses));

        let client = paws_release::GitHubReleaseClient::new(
            "octo".to_string(),
            "repo".to_string(),
            "t".to_string(),
        )
        .with_base_url_for_tests(format!("http://{addr}"));

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            publish_github_pages(&client, "main", &dir),
        )
        .await
        .expect("publish_github_pages should not hang");
        result.unwrap();

        server.await.unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }
}
