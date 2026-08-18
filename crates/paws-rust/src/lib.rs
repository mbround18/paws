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

/// A Rust project has a `Cargo.toml` at its root.
pub fn is_rust_project(dir: &Path) -> bool {
    dir.join("Cargo.toml").is_file()
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`)
/// for `source_dir`: `cargo fmt -- --check`, `cargo clippy`, `cargo build
/// --verbose`, `cargo test --verbose`, in that order — matching
/// `rustBuildAndTest`'s real step sequence and fail-fast behavior (each
/// step only runs if the previous one succeeded; `paws_dagger::core`
/// aborts the whole pipeline on the first non-zero exit).
pub fn dagger_pipeline_args(source_dir: &str) -> Vec<String> {
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

    push_exec(&["rustup", "component", "add", "rustfmt", "clippy"]);
    push_exec(&["cargo", "fmt", "--", "--check"]);
    push_exec(&["cargo", "clippy"]);
    push_exec(&["cargo", "build", "--verbose"]);
    push_exec(&["cargo", "test", "--verbose"]);

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
        let args = dagger_pipeline_args("/host/src");
        assert_eq!(args[0], "container");
        assert_eq!(args[1], "from");
        assert_eq!(args[2], "--address=rust:1-bookworm");
    }

    #[test]
    fn pipeline_runs_the_full_fmt_clippy_build_test_sequence_in_order() {
        let args = dagger_pipeline_args("/host/src");
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
}
