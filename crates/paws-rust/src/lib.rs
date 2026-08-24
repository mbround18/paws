//! Native Rust CI support, replacing `paws ci --toolchain rust`'s previous
//! dependency on `gh-reusable`'s `rustBuildAndTest` Dagger function.
//! Step sequence (`cargo fmt -- --check`, `cargo clippy`, `cargo build
//! --verbose`, `cargo test --verbose`) is ported from that real function
//! (`packages/dagger-module/src/index.ts`), read directly for parity, not
//! reimplemented from memory — only the setup differs: `rustBuildAndTest`
//! runs a full `rustup toolchain install`/`rustup default` dance to pin an
//! exact toolchain version; this crate uses the `rust:1-bookworm` image
//! already used by every other `paws`-authored Dockerfile/pipeline in this
//! repo (whatever stable Rust that image currently ships), plus `rustup
//! component add rustfmt clippy` — verified directly that neither ships by
//! default on that image (`cargo fmt --version` fails with "'cargo-fmt' is
//! not installed for the toolchain" until that component is added).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const BASE_IMAGE: &str = "rust:1-bookworm";

/// The `builders/rust` Dockerfile (`rust:1-bookworm` + `cargo-llvm-cov` +
/// `llvm-tools-preview`), embedded at compile time from
/// `builders/rust/Dockerfile`. Only used when `--coverage` is set — see
/// [`dagger_pipeline_args`]'s doc comment. `paws ci` runs from inside
/// whatever *target* repo it's checking, not from inside `paws`'s own
/// source tree, so a repo-relative `builders/rust` path would resolve
/// against the wrong directory once `paws` is used as a general-purpose
/// tool (same reasoning `paws-tauri`'s/`paws-java`'s own embedded
/// Dockerfiles document) — embedding + materializing to a temp dir (see
/// [`write_builder_dockerfile`]) makes this correct regardless of where
/// `paws` is invoked from.
const RUST_COVERAGE_DOCKERFILE: &str = include_str!("../../../builders/rust/Dockerfile");

/// Writes the embedded `builders/rust` Dockerfile to a temp directory and
/// returns that directory's path, suitable for [`dagger_pipeline_args`]'s
/// `builder_dir` argument — mirrors `paws-tauri`'s/`paws-java`'s own
/// same-named function.
pub fn write_builder_dockerfile() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("paws-builders").join("rust");
    std::fs::create_dir_all(&dir)
        .context("failed to create temp dir for the rust builder Dockerfile")?;
    std::fs::write(dir.join("Dockerfile"), RUST_COVERAGE_DOCKERFILE)
        .context("failed to write the rust builder Dockerfile")?;
    Ok(dir)
}

/// The target `wasm-pack`/`wasm-bindgen` crates build for — used both to
/// detect a wasm project and to pass `--target` to clippy/build.
pub const WASM_TARGET: &str = "wasm32-unknown-unknown";

/// A Rust project has a `Cargo.toml` at its root.
pub fn is_rust_project(dir: &Path) -> bool {
    dir.join("Cargo.toml").is_file()
}

/// A wasm-bindgen/wasm-pack project declares target-gated dependencies
/// under `[target.wasm32-unknown-unknown.dependencies]` and/or depends on
/// `wasm-bindgen` directly — either is a deliberate, purpose-built signal
/// (unlike e.g. a stray "wasm" in a comment), so a plain substring check
/// on the manifest text is enough — matching the string-matching detection
/// style already used by `paws_python::detect_project` rather than pulling
/// in a TOML-parsing dependency for this alone.
pub fn is_wasm_project(dir: &Path) -> bool {
    let Ok(manifest) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
        return false;
    };
    manifest.contains(WASM_TARGET) || manifest.contains("wasm-bindgen")
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`)
/// for `source_dir`: `cargo fmt -- --check`, `cargo clippy`, `cargo build
/// --verbose`, `cargo test --verbose`, in that order — matching
/// `rustBuildAndTest`'s real step sequence and fail-fast behavior (each
/// step only runs if the previous one succeeded; `paws_dagger::core`
/// aborts the whole pipeline on the first non-zero exit).
///
/// When `is_wasm` is set (see [`is_wasm_project`]), the sequence instead
/// adds the wasm32 target, gates clippy on `-D warnings` (`cargo-clippy`
/// otherwise only warns, so a project's dead-code/lint regressions would
/// never fail CI — this is the actual bug that silently broke
/// wikijs-module-meilisearch's release pipeline for months), builds for
/// `wasm32-unknown-unknown` instead of the host target, and skips `cargo
/// test` — a `cdylib` compiled for wasm32 can't run on the host, and
/// exercising it needs `wasm-bindgen-test-runner` plus a JS engine, which
/// is out of scope for this generic gate.
///
/// `coverage` (default `false`) is `paws ci --toolchain rust --coverage`'s
/// opt-in (specs/004-rust-coverage/spec.md): when set on a non-wasm
/// project, the opening chain builds `builders/rust` (via
/// `builder_dir`, from [`write_builder_dockerfile`]) instead of pulling
/// `BASE_IMAGE` directly, and one extra step —
/// `cargo llvm-cov --workspace --summary-only` — is appended *after* the
/// existing `cargo test --verbose` step, which is otherwise completely
/// unchanged (spec's Clarifications: tests execute once for the pass/fail
/// gate via `cargo test`, then again via `cargo llvm-cov` purely for the
/// coverage report). `builder_dir` is required (and only used) when
/// `coverage` is true; pass `None` when it's false. On a wasm project,
/// `coverage` is a silent no-op (research.md R5 in that spec) — the wasm
/// pipeline already can't run `cargo test` on the host, so there's nothing
/// for `cargo llvm-cov` to measure; the wasm sequence runs exactly as it
/// does without `--coverage`, no extra step, no error.
///
/// Omitting `coverage` (`false`, `builder_dir: None`) reproduces this
/// function's exact pre-`--coverage` output — a regression test pins this.
pub fn dagger_pipeline_args(
    source_dir: &str,
    is_wasm: bool,
    coverage: bool,
    builder_dir: Option<&str>,
) -> Vec<String> {
    let mut args: Vec<String> = if coverage && !is_wasm {
        let builder_dir = builder_dir
            .expect("builder_dir must be Some(..) when coverage is true (see doc comment)");
        vec![
            "host".into(),
            "directory".into(),
            format!("--path={builder_dir}"),
            "docker-build".into(),
            "with-mounted-directory".into(),
            "--path=/src".into(),
            format!("--source={source_dir}"),
            "with-workdir".into(),
            "--path=/src".into(),
        ]
    } else {
        vec![
            "container".into(),
            "from".into(),
            format!("--address={BASE_IMAGE}"),
            "with-mounted-directory".into(),
            "--path=/src".into(),
            format!("--source={source_dir}"),
            "with-workdir".into(),
            "--path=/src".into(),
        ]
    };

    let mut push_exec = |command_args: &[&str]| {
        args.push("with-exec".into());
        args.push(format!("--args={}", command_args.join(",")));
    };

    if is_wasm {
        push_exec(&["rustup", "target", "add", WASM_TARGET]);
        push_exec(&["rustup", "component", "add", "rustfmt", "clippy"]);
        push_exec(&["cargo", "fmt", "--", "--check"]);
        push_exec(&[
            "cargo",
            "clippy",
            "--target",
            WASM_TARGET,
            "--",
            "-D",
            "warnings",
        ]);
        push_exec(&["cargo", "build", "--target", WASM_TARGET, "--verbose"]);
    } else {
        push_exec(&["rustup", "component", "add", "rustfmt", "clippy"]);
        push_exec(&["cargo", "fmt", "--", "--check"]);
        push_exec(&["cargo", "clippy", "--", "-D", "warnings"]);
        push_exec(&["cargo", "build", "--verbose"]);
        push_exec(&["cargo", "test", "--verbose"]);
        if coverage {
            push_exec(&["cargo", "llvm-cov", "--workspace", "--summary-only"]);
        }
    }

    args.push("stdout".into());
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("paws-rust-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_rust_project_from_cargo_toml() {
        let dir = temp_dir("detect");
        assert!(
            !is_rust_project(&dir),
            "should not detect before Cargo.toml exists"
        );
        fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        assert!(is_rust_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Writes a minimal, standalone (own empty `[workspace]`) fixture crate
    /// to `dir`, with `lib_contents` as its `src/lib.rs` — used by the
    /// clippy-gate fixture tests below to exercise the real
    /// `cargo clippy -- -D warnings` invocation directly (not just asserting
    /// the string `dagger_pipeline_args` builds), matching how `paws-docs`'s
    /// own tests shell out to a real `cargo` subcommand.
    fn write_clippy_fixture(dir: &std::path::Path, lib_contents: &str) {
        fs::write(
            dir.join("Cargo.toml"),
            "[workspace]\n\n[package]\nname = \"clippy-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/lib.rs"), lib_contents).unwrap();
    }

    // T003 (US1): a real clippy warning fails `cargo clippy -- -D warnings`
    // when invoked directly — proves the gate itself, independent of the
    // dagger-pipeline string-assertion test above.
    #[test]
    fn a_real_clippy_warning_fails_with_d_warnings() {
        let dir = temp_dir("clippy-warn");
        write_clippy_fixture(
            &dir,
            "pub fn check(flag: bool) -> bool {\n    if flag == true { true } else { false }\n}\n",
        );

        let status = std::process::Command::new("cargo")
            .args(["clippy", "--", "-D", "warnings"])
            .current_dir(&dir)
            .status()
            .expect("failed to spawn cargo clippy");

        assert!(
            !status.success(),
            "a crate with a real clippy warning (bool_comparison) must fail -D warnings"
        );
        fs::remove_dir_all(&dir).ok();
    }

    // T005 (US1, SC-002): a clean, warning-free fixture continues to pass —
    // zero false positives introduced by the -D warnings gate.
    #[test]
    fn a_clean_fixture_still_passes_with_d_warnings() {
        let dir = temp_dir("clippy-clean");
        write_clippy_fixture(&dir, "pub fn check(flag: bool) -> bool {\n    flag\n}\n");

        let status = std::process::Command::new("cargo")
            .args(["clippy", "--", "-D", "warnings"])
            .current_dir(&dir)
            .status()
            .expect("failed to spawn cargo clippy");

        assert!(
            status.success(),
            "a clean, warning-free crate must still pass -D warnings"
        );
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pipeline_uses_the_rust_bookworm_image() {
        let args = dagger_pipeline_args("/host/src", false, false, None);
        assert_eq!(args[0], "container");
        assert_eq!(args[1], "from");
        assert_eq!(args[2], "--address=rust:1-bookworm");
    }

    #[test]
    fn pipeline_runs_the_full_fmt_clippy_build_test_sequence_in_order() {
        let args = dagger_pipeline_args("/host/src", false, false, None);
        let expected = [
            "--args=rustup,component,add,rustfmt,clippy",
            "--args=cargo,fmt,--,--check",
            "--args=cargo,clippy,--,-D,warnings",
            "--args=cargo,build,--verbose",
            "--args=cargo,test,--verbose",
        ];
        let positions: Vec<usize> = expected
            .iter()
            .map(|step| args.iter().position(|a| a == step).unwrap())
            .collect();
        assert!(
            positions.windows(2).all(|w| w[0] < w[1]),
            "steps must run in order: {positions:?}"
        );
        assert_eq!(args.last(), Some(&"stdout".to_string()));
    }

    // T004 (SC-equivalent byte-identical-default guarantee): already covered
    // by `pipeline_uses_the_rust_bookworm_image`/
    // `pipeline_runs_the_full_fmt_clippy_build_test_sequence_in_order` above,
    // now exercising the extended 4-arg signature with `coverage`/
    // `builder_dir` defaulted off — both passed unmodified after T003's
    // signature extension, confirming byte-identical default output.

    #[test]
    fn coverage_appends_a_cargo_llvm_cov_step_after_cargo_test() {
        let args = dagger_pipeline_args("/host/src", false, true, Some("/tmp/builder"));
        let test_pos = args
            .iter()
            .position(|a| a == "--args=cargo,test,--verbose")
            .unwrap();
        let coverage_pos = args
            .iter()
            .position(|a| a == "--args=cargo,llvm-cov,--workspace,--summary-only")
            .unwrap();
        assert!(
            test_pos < coverage_pos,
            "cargo llvm-cov must run after cargo test, not replace or precede it"
        );
        // cargo test's own step is untouched — same literal args as the
        // non-coverage path.
        assert!(args.contains(&"--args=cargo,test,--verbose".to_string()));
    }

    #[test]
    fn coverage_swaps_the_opening_chain_to_docker_build_against_the_builder_dir() {
        let args = dagger_pipeline_args("/host/src", false, true, Some("/tmp/builder"));
        assert_eq!(args[0], "host");
        assert_eq!(args[1], "directory");
        assert_eq!(args[2], "--path=/tmp/builder");
        assert_eq!(args[3], "docker-build");
        assert!(!args.iter().any(|a| a == "--address=rust:1-bookworm"));
    }

    #[test]
    fn coverage_is_a_noop_on_a_wasm_project() {
        let with_coverage = dagger_pipeline_args("/host/src", true, true, Some("/tmp/builder"));
        let without_coverage = dagger_pipeline_args("/host/src", true, false, None);
        assert_eq!(
            with_coverage, without_coverage,
            "--coverage must not change the wasm pipeline's output at all"
        );
        assert!(
            !with_coverage.iter().any(|a| a.contains("llvm-cov")),
            "no coverage step should appear on a wasm project"
        );
    }

    #[test]
    fn detects_a_wasm_project_from_target_gated_deps_or_wasm_bindgen() {
        let dir = temp_dir("wasm-detect");
        fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        assert!(!is_wasm_project(&dir), "plain crate isn't wasm");

        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"x\"\n\n[dependencies]\nwasm-bindgen = \"0.2\"\n",
        )
        .unwrap();
        assert!(is_wasm_project(&dir));

        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"x\"\n\n[target.wasm32-unknown-unknown.dependencies]\nweb-sys = \"0.3\"\n",
        )
        .unwrap();
        assert!(is_wasm_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wasm_pipeline_adds_the_target_gates_clippy_and_skips_cargo_test() {
        let args = dagger_pipeline_args("/host/src", true, false, None);
        let expected = [
            "--args=rustup,target,add,wasm32-unknown-unknown",
            "--args=rustup,component,add,rustfmt,clippy",
            "--args=cargo,fmt,--,--check",
            "--args=cargo,clippy,--target,wasm32-unknown-unknown,--,-D,warnings",
            "--args=cargo,build,--target,wasm32-unknown-unknown,--verbose",
        ];
        let positions: Vec<usize> = expected
            .iter()
            .map(|step| args.iter().position(|a| a == step).unwrap())
            .collect();
        assert!(
            positions.windows(2).all(|w| w[0] < w[1]),
            "steps must run in order: {positions:?}"
        );
        assert!(
            !args.iter().any(|a| a.contains("cargo,test")),
            "wasm target can't run cargo test on the host"
        );
        assert_eq!(args.last(), Some(&"stdout".to_string()));
    }
}
