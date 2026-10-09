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

use paws_core::{Base, ContainerOptions, container::SOURCE_ROOT};
use std::path::Path;

use anyhow::Result;

pub const BASE_IMAGE: &str = "rust:1-bookworm";

/// The `builders/rust` Dockerfile (`rust:1-bookworm` + `cargo-llvm-cov` +
/// `llvm-tools-preview`), embedded at compile time from
/// `builders/rust/Dockerfile`. Only used when `--coverage` is set — see
/// [`dagger_pipeline_args`]'s doc comment. `paws ci` runs from inside
/// whatever *target* repo it's checking, not from inside `paws`'s own
/// source tree, so a repo-relative `builders/rust` path would resolve
/// against the wrong directory once `paws` is used as a general-purpose
/// tool; embedding the text and writing it into the build context (see
/// [`paws_core::Base::Dockerfile`]) makes this correct regardless of where
/// `paws` is invoked from.
pub const RUST_COVERAGE_DOCKERFILE: &str = include_str!("../../../builders/rust/Dockerfile");

/// How `cargo` is invoked: the whole workspace or just the root package,
/// with any members left out and any extra flags the project's own CI
/// passes (`--all-targets`, `--no-default-features`, ...).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CargoOptions {
    /// `--workspace` on clippy/build/test and `--all` on fmt.
    pub workspace: bool,
    /// `--exclude <member>` for each, which cargo only accepts together with
    /// `--workspace`, so any entry implies it.
    pub exclude: Vec<String>,
    /// Appended to clippy (before its `--`), build and test.
    pub args: Vec<String>,
}

impl CargoOptions {
    const fn workspace(&self) -> bool {
        self.workspace || !self.exclude.is_empty()
    }

    /// `cargo <subcommand> [--workspace] [--exclude m]* [args]* <tail>`
    fn command(&self, subcommand: &str, tail: &[&str]) -> Vec<String> {
        let mut command = vec!["cargo".to_string(), subcommand.to_string()];
        if self.workspace() {
            command.push("--workspace".into());
        }
        for member in &self.exclude {
            command.push("--exclude".into());
            command.push(member.clone());
        }
        command.extend(self.args.iter().cloned());
        command.extend(tail.iter().map(ToString::to_string));
        command
    }

    fn fmt(&self) -> Vec<String> {
        let mut command = vec!["cargo".to_string(), "fmt".to_string()];
        if self.workspace() {
            command.push("--all".into());
        }
        command.extend(["--".to_string(), "--check".to_string()]);
        command
    }
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
/// The source reaches the container the way `paws run` sends it: filtered
/// on the host by `.gitignore` (so a workstation's `target/` is neither
/// uploaded nor seen), built into an image from `image` plus
/// `container`'s packages and setup, with its caches and environment
/// applied. `cargo` takes the shape `cargo` describes; `container.export`
/// swaps the final stdout for a directory export.
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
/// project, the image is built from `builders/rust`
/// ([`RUST_COVERAGE_DOCKERFILE`]) instead of `image`, and one extra step —
/// `cargo llvm-cov --workspace --summary-only` — is appended *after* the
/// existing `cargo test --verbose` step, which is otherwise completely
/// unchanged (spec's Clarifications: tests execute once for the pass/fail
/// gate via `cargo test`, then again via `cargo llvm-cov` purely for the
/// coverage report). On a wasm project, `coverage` is a silent no-op
/// (research.md R5 in that spec) — the wasm pipeline already can't run
/// `cargo test` on the host, so there's nothing for `cargo llvm-cov` to
/// measure.
pub fn dagger_pipeline_args(
    source_dir: &str,
    is_wasm: bool,
    coverage: bool,
    image: &str,
    cargo: &CargoOptions,
    container: &ContainerOptions,
) -> Result<Vec<String>> {
    let base = if coverage && !is_wasm {
        Base::Dockerfile(RUST_COVERAGE_DOCKERFILE)
    } else {
        Base::Image(image)
    };
    let pipeline = container.open(source_dir, &base)?.workdir(SOURCE_ROOT);

    let pipeline = if is_wasm {
        pipeline
            .exec(["rustup", "target", "add", WASM_TARGET])
            .exec(["rustup", "component", "add", "rustfmt", "clippy"])
            .exec(cargo.fmt())
            .exec(cargo.command("clippy", &["--target", WASM_TARGET, "--", "-D", "warnings"]))
            .exec(cargo.command("build", &["--target", WASM_TARGET, "--verbose"]))
    } else {
        pipeline
            .exec(["rustup", "component", "add", "rustfmt", "clippy"])
            .exec(cargo.fmt())
            .exec(cargo.command("clippy", &["--", "-D", "warnings"]))
            .exec(cargo.command("build", &["--verbose"]))
            .exec(cargo.command("test", &["--verbose"]))
            .exec_if(
                coverage,
                ["cargo", "llvm-cov", "--workspace", "--summary-only"],
            )
    };
    Ok(container.finish(pipeline))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        paws_core::test_support::scratch_dir("rust", name)
    }

    fn args(is_wasm: bool, coverage: bool) -> Vec<String> {
        dagger_pipeline_args(
            "/host/src",
            is_wasm,
            coverage,
            BASE_IMAGE,
            &CargoOptions::default(),
            &ContainerOptions::default(),
        )
        .unwrap()
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
    fn write_clippy_fixture(dir: &Path, lib_contents: &str) {
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
    fn pipeline_builds_the_bookworm_image_from_the_filtered_source() {
        let args = args(false, false);
        assert_eq!(
            &args[..4],
            &["host", "directory", "--path=/host/src", "--gitignore"]
        );
        assert_eq!(args[4], "--exclude=.git");
        assert!(
            args.iter()
                .any(|a| a.starts_with("--contents=FROM rust:1-bookworm\n"))
        );
        assert!(!args.iter().any(|a| a.starts_with("--build-args=")));
        assert!(args.contains(&"--path=/src".to_string()));
    }

    #[test]
    fn pipeline_runs_the_full_fmt_clippy_build_test_sequence_in_order() {
        let args = args(false, false);
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

    #[test]
    fn workspace_excludes_and_extra_args_reach_every_cargo_step() {
        let cargo = CargoOptions {
            workspace: true,
            exclude: vec!["fathom-desktop".into()],
            args: vec!["--all-targets".into()],
        };
        let args = dagger_pipeline_args(
            "/host/src",
            false,
            false,
            BASE_IMAGE,
            &cargo,
            &ContainerOptions::default(),
        )
        .unwrap();
        assert!(args.contains(&"--args=cargo,fmt,--all,--,--check".to_string()));
        assert!(args.contains(
            &"--args=cargo,clippy,--workspace,--exclude,fathom-desktop,--all-targets,--,-D,warnings"
                .to_string()
        ));
        assert!(
            args.contains(
                &"--args=cargo,build,--workspace,--exclude,fathom-desktop,--all-targets,--verbose"
                    .to_string()
            )
        );
        assert!(
            args.contains(
                &"--args=cargo,test,--workspace,--exclude,fathom-desktop,--all-targets,--verbose"
                    .to_string()
            )
        );
    }

    #[test]
    fn an_exclude_alone_implies_the_workspace() {
        let cargo = CargoOptions {
            exclude: vec!["ui".into()],
            ..Default::default()
        };
        assert_eq!(
            cargo.command("build", &["--verbose"]),
            [
                "cargo",
                "build",
                "--workspace",
                "--exclude",
                "ui",
                "--verbose"
            ]
        );
    }

    #[test]
    fn container_options_shape_the_image_and_the_ending() {
        let container = ContainerOptions {
            apt_packages: vec!["cmake".into()],
            env: vec![("CARGO_BUILD_JOBS".into(), "4".into())],
            caches: vec![paws_core::CacheMount {
                name: "t".into(),
                path: "/src/target".into(),
            }],
            export: Some(paws_core::Export {
                path: "/src/target/release".into(),
                destination: "/host/out".into(),
            }),
            ..Default::default()
        };
        let args = dagger_pipeline_args(
            "/host/src",
            false,
            false,
            BASE_IMAGE,
            &CargoOptions::default(),
            &container,
        )
        .unwrap();
        assert!(args.iter().any(|a| a.starts_with("--contents=")
            && a.contains("apt-get install")
            && a.contains(" cmake ")));
        assert!(args.contains(&"--cache=t".to_string()));
        assert!(args.contains(&"--name=CARGO_BUILD_JOBS".to_string()));
        assert_eq!(args.last().unwrap(), "--path=/host/out");
    }

    #[test]
    fn coverage_appends_a_cargo_llvm_cov_step_after_cargo_test() {
        let args = args(false, true);
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
    fn coverage_builds_from_the_embedded_rust_builder_dockerfile() {
        let args = args(false, true);
        let dockerfile = args
            .iter()
            .find(|a| a.starts_with("--contents=") && a.contains("FROM "))
            .unwrap();
        assert!(dockerfile.contains("cargo install cargo-llvm-cov"));
        assert!(dockerfile.ends_with("WORKDIR /src\nCOPY . /src"));
        assert!(
            args.iter()
                .any(|a| a.starts_with("--build-args=BUILDER_VERSION="))
        );
    }

    #[test]
    fn coverage_is_a_noop_on_a_wasm_project() {
        let with_coverage = args(true, true);
        let without_coverage = args(true, false);
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
        let args = args(true, false);
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
