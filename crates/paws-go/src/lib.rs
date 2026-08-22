//! Native Go CI support for `paws ci --toolchain go`. Unlike `paws-python`/
//! `paws-rust`, there's no `gh-reusable` `goBuildAndTest` Dagger function to
//! port for parity — `gh-reusable` only ever had `setupGo` (a container
//! setup step with no build/test steps of its own; see
//! `packages/dagger-module/src/index.ts`). This crate's step sequence
//! (`go build ./...`, `go vet ./...`, `go test ./...`) is a new, native
//! implementation following ordinary Go project conventions rather than a
//! port, deliberately minimal to match `paws-python`'s scope (no formatting
//! gate — `gofmt -l` doesn't itself fail on unformatted code the way
//! `cargo fmt -- --check` does, and wiring a `test -z "$(gofmt -l .)"`
//! shell one-liner through `dagger core`'s comma-joined `--args` would risk
//! exactly the CSV-parsing fragility `paws-audit` hit and moved away from —
//! not worth it for a first cut).

use std::path::Path;

pub const BASE_IMAGE: &str = "golang:1-bookworm";

/// A Go project has a `go.mod` at its root — the file every `go` subcommand
/// used here (`build`/`vet`/`test`) requires to resolve the module.
pub fn is_go_project(dir: &Path) -> bool {
    dir.join("go.mod").is_file()
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`)
/// for `source_dir`: `go build ./...`, `go vet ./...`, `go test ./...`, in
/// that order and fail-fast (each step only runs if the previous one
/// succeeded; `paws_dagger::core` aborts the whole pipeline on the first
/// non-zero exit) — `./...` matches every package in the module, the same
/// "build/test everything" scope `cargo build`/`cargo test` give for free
/// in `paws-rust`.
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

    push_exec(&["go", "build", "./..."]);
    push_exec(&["go", "vet", "./..."]);
    push_exec(&["go", "test", "./..."]);

    args.push("stdout".into());
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("paws-go-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_go_project_from_go_mod() {
        let dir = temp_dir("detect");
        assert!(
            !is_go_project(&dir),
            "should not detect before go.mod exists"
        );
        fs::write(dir.join("go.mod"), "module example.com/x\n\ngo 1.23\n").unwrap();
        assert!(is_go_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pipeline_uses_the_default_go_image() {
        let args = dagger_pipeline_args("/host/src");
        assert_eq!(args[0], "container");
        assert_eq!(args[1], "from");
        assert_eq!(args[2], "--address=golang:1-bookworm");
    }

    #[test]
    fn pipeline_runs_the_full_build_vet_test_sequence_in_order() {
        let args = dagger_pipeline_args("/host/src");
        assert_eq!(
            args,
            vec![
                "container".to_string(),
                "from".to_string(),
                "--address=golang:1-bookworm".to_string(),
                "with-mounted-directory".to_string(),
                "--path=/src".to_string(),
                "--source=/host/src".to_string(),
                "with-workdir".to_string(),
                "--path=/src".to_string(),
                "with-exec".to_string(),
                "--args=go,build,./...".to_string(),
                "with-exec".to_string(),
                "--args=go,vet,./...".to_string(),
                "with-exec".to_string(),
                "--args=go,test,./...".to_string(),
                "stdout".to_string(),
            ]
        );
    }
}
