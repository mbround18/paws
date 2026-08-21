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

use std::path::Path;

pub const BASE_IMAGE: &str = "rust:1-bookworm";

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
pub fn dagger_pipeline_args(source_dir: &str, is_wasm: bool) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "container".into(),
        "from".into(),
        format!("--address={BASE_IMAGE}"),
        "with-mounted-directory".into(),
        "--path=/src".into(),
        format!("--source={source_dir}"),
        "with-workdir".into(),
        "--path=/src".into(),
    ];

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
        push_exec(&["cargo", "clippy"]);
        push_exec(&["cargo", "build", "--verbose"]);
        push_exec(&["cargo", "test", "--verbose"]);
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

    #[test]
    fn pipeline_uses_the_rust_bookworm_image() {
        let args = dagger_pipeline_args("/host/src", false);
        assert_eq!(args[0], "container");
        assert_eq!(args[1], "from");
        assert_eq!(args[2], "--address=rust:1-bookworm");
    }

    #[test]
    fn pipeline_runs_the_full_fmt_clippy_build_test_sequence_in_order() {
        let args = dagger_pipeline_args("/host/src", false);
        let expected = [
            "--args=rustup,component,add,rustfmt,clippy",
            "--args=cargo,fmt,--,--check",
            "--args=cargo,clippy",
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
        let args = dagger_pipeline_args("/host/src", true);
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
