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
        assert!(docs_dir.is_dir(), "expected {docs_dir:?} to exist after a successful build");
    }

    #[tokio::test]
    async fn rerunning_against_an_unchanged_workspace_is_still_successful() {
        let workspace = repo_root();
        build_docs(&workspace).await.unwrap();
        // Re-running is the idempotency contract itself: no error, no required cleanup.
        let docs_dir = build_docs(&workspace).await.unwrap();
        assert!(docs_dir.is_dir());
    }
}
