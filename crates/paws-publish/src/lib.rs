//! Native `paws publish --target rust-crate` support, replacing
//! `gh-reusable`'s real `publishRustCrate` Dagger function
//! (`packages/dagger-module/src/index.ts`) — read directly for parity, not
//! reimplemented from memory. Only the `rust-crate` target is ported here;
//! `gh-reusable`'s `publish.yaml` also supports `node`/`helm-chart`
//! targets, neither of which any real, currently-active repo actually
//! needs from `paws` yet (confirmed against `mbround18/game-server-management`,
//! the repo whose `docker-release.yaml` still calls this — its 10
//! `libs/*` crates only ever use `target: rust-crate`).
//!
//! Step sequence — `cargo check`, `cargo test`, `cargo package`, `cargo
//! publish` — is `gh-reusable`'s own real sequence, fail-fast, against the
//! same `rust:1-bookworm` image `paws-rust` already uses (duplicated
//! rather than shared, matching how every other per-toolchain crate in
//! this workspace independently owns its own pipeline builder). The one
//! deliberate simplification: `gh-reusable`'s `--version` override
//! parameter only ever affected *report text*, never the actual
//! `cargo publish` command (which always publishes whatever version is in
//! `Cargo.toml` regardless) — not ported, since carrying it forward would
//! just be complexity with no real effect to preserve.
//!
//! `crates.io` is Cargo's own default registry — `cargo publish` reads a
//! bare `CARGO_REGISTRY_TOKEN` for it with no `--registry` flag needed at
//! all. Any other named registry needs `--registry <name>` plus Cargo's
//! own (not `paws-docker`'s generic per-registry scheme) `CARGO_REGISTRIES_
//! <NAME>_TOKEN` convention — see [`token_env_var`].
//!
//! [`find_workspace_root`] exists because of a real bug this crate's
//! design has to route around, not just port past: `gh-reusable`'s own
//! `publishRustCrate` mounts *only* the crate's own subdirectory (the
//! `source: Directory` it's given), which breaks outright for any crate
//! using Cargo's `workspace = true` field inheritance (e.g. `[lints]
//! workspace = true`) — confirmed for real against
//! `mbround18/game-server-management`'s actual `libs/env-parse` crate
//! (which uses exactly this), and confirmed this isn't hypothetical: its
//! own tag-triggered `publish-crates` CI runs have failed on every real
//! attempt so far (`gh run list`/`gh api .../logs`, 2026-08-22), all 10
//! `libs/*` crates failing the same way. This crate mounts the real
//! workspace root instead whenever one exists, `with-workdir`ing into the
//! crate's own subpath — confirmed for real this fixes `cargo check`
//! against a real copy of `libs/env-parse`.

use paws_core::Pipeline;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const BASE_IMAGE: &str = "rust:1-bookworm";

pub const DEFAULT_REGISTRY: &str = "crates.io";

/// A Rust crate has a `Cargo.toml` at its root — same detection paws-rust
/// already uses for `--toolchain rust`.
pub fn is_rust_crate(dir: &Path) -> bool {
    dir.join("Cargo.toml").is_file()
}

fn declares_workspace(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("Cargo.toml"))
        .is_ok_and(|s| s.lines().any(|line| line.trim() == "[workspace]"))
}

/// Walks up from `dir` looking for the nearest ancestor whose `Cargo.toml`
/// declares `[workspace]` — see this module's doc comment for why this
/// matters. Stops at the first `.git` directory found (the repository
/// boundary — checking that directory's own `Cargo.toml` one last time
/// first) so an unrelated ancestor directory elsewhere on disk (e.g. a
/// developer's home directory that happens to contain some other
/// `Cargo.toml`) is never mistaken for `dir`'s real workspace. Returns
/// `None` if no ancestor declares one, **or if `dir` already declares its
/// own** (checked first, before ever walking up) — a real, deliberate
/// pattern confirmed against this very repo's own `examples/*-fixture`
/// crates: an empty `[workspace]` alongside `[package]` in the same
/// `Cargo.toml` is exactly how a crate opts *out* of an enclosing
/// workspace it happens to be nested under on disk (`examples/rust-
/// fixture` is deliberately not a member of `paws`'s own root workspace).
/// Walking straight past that self-declaration to `paws`'s real workspace
/// root — genuinely reproduced before this check existed — would silently
/// mount the wrong, much larger directory instead of the standalone crate
/// actually being published.
pub fn find_workspace_root(dir: &Path) -> Option<PathBuf> {
    if declares_workspace(dir) {
        return None;
    }
    let mut current = dir.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return declares_workspace(&current).then_some(current);
        }
        let parent = current.parent()?.to_path_buf();
        if declares_workspace(&parent) {
            return Some(parent);
        }
        current = parent;
    }
}

/// Reads the crate's own name from its `[package]` section — enough to
/// report what's being published without pulling in a TOML-parsing
/// dependency for this alone (matching `paws_go::module_name`'s plain
/// text-scan style for the same reason).
pub fn read_crate_name(dir: &Path) -> Result<String> {
    let manifest = std::fs::read_to_string(dir.join("Cargo.toml"))
        .with_context(|| format!("failed to read Cargo.toml in {}", dir.display()))?;

    let mut in_package = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed == "[package]" {
            in_package = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_package = false;
            continue;
        }
        if !in_package {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("name") else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let name = value.trim().trim_matches('"').to_string();
        if !name.is_empty() {
            return Ok(name);
        }
    }

    anyhow::bail!("no [package] name found in Cargo.toml in {}", dir.display())
}

/// The env var `cargo publish` itself reads the token from for `registry`.
/// `crates.io` (Cargo's own default registry) is `CARGO_REGISTRY_TOKEN`,
/// no registry name involved; any other named registry is Cargo's own
/// `CARGO_REGISTRIES_<NAME>_TOKEN` convention (uppercased, every
/// non-alphanumeric character replaced with `_`) — this is Cargo's real
/// mechanism, not `paws-docker::registry_token_env_var`'s generic
/// per-registry scheme, which `cargo` itself doesn't read.
pub fn token_env_var(registry: &str) -> String {
    if registry == DEFAULT_REGISTRY {
        return "CARGO_REGISTRY_TOKEN".to_string();
    }
    let sanitized: String = registry
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("CARGO_REGISTRIES_{}_TOKEN", sanitized.to_ascii_uppercase())
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`):
/// mounts `mount_dir` (the crate's own directory for a standalone crate, or
/// its real workspace root per [`find_workspace_root`] for a workspace
/// member — the caller resolves which), runs from `workdir` inside that
/// mount (`/src` for a standalone crate, `/src/<relative path to the
/// crate>` for a workspace member), then `cargo check`, `cargo test`,
/// `cargo package`, fail-fast, against [`BASE_IMAGE`]. When `dry_run` is
/// set, stops there — matching `gh-reusable`'s `publish: false` build/
/// package-only mode, for verifying a crate is publish-ready without
/// actually publishing it. Otherwise injects `token` as
/// [`token_env_var`]'s env var (via `with-secret-variable`, never printed)
/// and runs `cargo publish`, adding `--registry <registry>` only when it
/// isn't [`DEFAULT_REGISTRY`] (crates.io needs no explicit flag — it's
/// Cargo's default).
pub fn dagger_pipeline_args(
    mount_dir: &str,
    workdir: &str,
    registry: &str,
    token_env_var: &str,
    dry_run: bool,
) -> Vec<String> {
    let mut pipeline = Pipeline::from_image(BASE_IMAGE)
        .mount("/src", mount_dir)
        .workdir(workdir)
        .exec(["cargo", "check"])
        .exec(["cargo", "test"])
        .exec(["cargo", "package"]);

    if !dry_run {
        pipeline = pipeline.secret_env(token_env_var);
        pipeline = if registry == DEFAULT_REGISTRY {
            pipeline.exec(["cargo", "publish"])
        } else {
            pipeline.exec(["cargo", "publish", "--registry", registry])
        };
    }

    pipeline.stdout()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        paws_core::test_support::scratch_dir("publish", name)
    }

    #[test]
    fn detects_rust_crate_from_cargo_toml() {
        let dir = temp_dir("detect");
        assert!(!is_rust_crate(&dir));
        fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        assert!(is_rust_crate(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_the_crate_name_from_the_package_section() {
        let dir = temp_dir("name");
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"env-parse\"\nversion = \"0.1.0\"\n\n[dependencies]\nname = \"not-this-one\"\n",
        )
        .unwrap();
        assert_eq!(read_crate_name(&dir).unwrap(), "env-parse");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_when_no_cargo_toml() {
        let dir = temp_dir("missing");
        assert!(read_crate_name(&dir).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn crates_io_uses_the_bare_cargo_registry_token_var() {
        assert_eq!(token_env_var("crates.io"), "CARGO_REGISTRY_TOKEN");
    }

    #[test]
    fn a_named_registry_uses_cargos_own_registries_token_convention() {
        assert_eq!(
            token_env_var("myco-internal"),
            "CARGO_REGISTRIES_MYCO_INTERNAL_TOKEN"
        );
    }

    #[test]
    fn pipeline_runs_check_test_package_publish_in_order_for_crates_io() {
        let args = dagger_pipeline_args(
            "/host/src",
            "/src",
            "crates.io",
            "CARGO_REGISTRY_TOKEN",
            false,
        );
        assert_eq!(
            args,
            vec![
                "container".to_string(),
                "from".to_string(),
                "--address=rust:1-bookworm".to_string(),
                "with-mounted-directory".to_string(),
                "--path=/src".to_string(),
                "--source=/host/src".to_string(),
                "with-workdir".to_string(),
                "--path=/src".to_string(),
                "with-exec".to_string(),
                "--args=cargo,check".to_string(),
                "with-exec".to_string(),
                "--args=cargo,test".to_string(),
                "with-exec".to_string(),
                "--args=cargo,package".to_string(),
                "with-secret-variable".to_string(),
                "--name=CARGO_REGISTRY_TOKEN".to_string(),
                "--secret=env:CARGO_REGISTRY_TOKEN".to_string(),
                "with-exec".to_string(),
                "--args=cargo,publish".to_string(),
                "stdout".to_string(),
            ]
        );
    }

    #[test]
    fn pipeline_uses_a_workspace_member_subpath_as_the_workdir() {
        let args = dagger_pipeline_args(
            "/host/repo",
            "/src/libs/env-parse",
            "crates.io",
            "CARGO_REGISTRY_TOKEN",
            false,
        );
        assert!(args.contains(&"--source=/host/repo".to_string()));
        assert!(args.contains(&"--path=/src/libs/env-parse".to_string()));
    }

    #[test]
    fn pipeline_adds_a_registry_flag_for_a_non_default_registry() {
        let args = dagger_pipeline_args(
            "/host/src",
            "/src",
            "myco-internal",
            "CARGO_REGISTRIES_MYCO_INTERNAL_TOKEN",
            false,
        );
        assert!(
            args.contains(&"--args=cargo,publish,--registry,myco-internal".to_string()),
            "{args:?}"
        );
    }

    #[test]
    fn dry_run_stops_after_packaging_no_secret_or_publish_step() {
        let args = dagger_pipeline_args(
            "/host/src",
            "/src",
            "crates.io",
            "CARGO_REGISTRY_TOKEN",
            true,
        );
        assert!(!args.iter().any(|a| a == "with-secret-variable"));
        assert!(!args.iter().any(|a| a.contains("publish")));
        assert_eq!(args.last(), Some(&"stdout".to_string()));
    }

    #[test]
    fn find_workspace_root_finds_the_ancestor_declaring_workspace() {
        let root = temp_dir("ws-root");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"libs/*\"]\n",
        )
        .unwrap();
        let member = root.join("libs/env-parse");
        fs::create_dir_all(&member).unwrap();
        fs::write(
            member.join("Cargo.toml"),
            "[package]\nname = \"env-parse\"\n",
        )
        .unwrap();

        assert_eq!(find_workspace_root(&member), Some(root.clone()));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_workspace_root_returns_none_for_a_standalone_crate() {
        let dir = temp_dir("standalone");
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(dir.join("Cargo.toml"), "[package]\nname = \"solo\"\n").unwrap();

        assert_eq!(find_workspace_root(&dir), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn find_workspace_root_returns_none_when_dir_itself_declares_an_empty_workspace() {
        // Real, reproduced bug this guards: a crate with its own empty
        // [workspace] (this repo's own examples/rust-fixture pattern)
        // deliberately opts *out* of any enclosing workspace it's nested
        // under on disk. Walking straight past that self-declaration to
        // find the *outer* real workspace (an ancestor .git repo with its
        // own real [workspace]) would silently mount the wrong, much
        // larger directory instead of this standalone crate.
        let outer = temp_dir("outer-real-workspace");
        fs::create_dir_all(outer.join(".git")).unwrap();
        fs::write(
            outer.join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/*\"]\n",
        )
        .unwrap();
        let nested = outer.join("examples/rust-fixture");
        fs::create_dir_all(&nested).unwrap();
        fs::write(
            nested.join("Cargo.toml"),
            "[workspace]\n\n[package]\nname = \"rust-fixture\"\n",
        )
        .unwrap();

        assert_eq!(find_workspace_root(&nested), None);
        fs::remove_dir_all(&outer).unwrap();
    }
}
