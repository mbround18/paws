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
//!
//! Two `docs/ROADMAP.md` "Go" row variants:
//! - `Go + C/C++ (cgo)` needs **no code here at all** — confirmed for real
//!   against `golang:1-bookworm`, `CGO_ENABLED=1` is already the image's
//!   default and it already ships `gcc`, so a package with `import "C"`
//!   already builds/tests through the exact same plain pipeline below.
//!   `examples/go-cgo-fixture` exists purely to prove that, not because
//!   the pipeline branches on it.
//! - `Go + WebAssembly` genuinely does need a different pipeline: a
//!   `GOOS=js GOARCH=wasm` cross-compile has no way to actually *run* the
//!   resulting binary in this container (no JS engine), so `go test` is
//!   skipped — same rationale as `paws-rust::is_wasm_project`'s wasm path
//!   skipping `cargo test` for a `cdylib` that can't run on the host either.

use std::path::{Path, PathBuf};

pub const BASE_IMAGE: &str = "golang:1-bookworm";

/// A Go project has a `go.mod` at its root — the file every `go` subcommand
/// used here (`build`/`vet`/`test`) requires to resolve the module.
pub fn is_go_project(dir: &Path) -> bool {
    dir.join("go.mod").is_file()
}

/// Every `.go` file under `dir`, recursing into subdirectories but skipping
/// `vendor/` (third-party code, not this module's own signal) and hidden
/// directories (`.git`, etc.) — used by [`is_wasm_project`] to scan real
/// source rather than a single manifest the way `paws-rust`'s
/// `Cargo.toml`-only scan can.
fn go_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name == "vendor" || name.starts_with('.') {
                continue;
            }
            files.extend(go_files(&path));
        } else if path.extension().and_then(|e| e.to_str()) == Some("go") {
            files.push(path);
        }
    }
    files
}

/// A Go/WebAssembly project imports `syscall/js` — the standard package for
/// JS interop when compiled to the `js`/`wasm` target, and a deliberate,
/// purpose-built signal (checked with its surrounding quotes, as it
/// appears in a real import statement) — the same detection style
/// `paws_rust::is_wasm_project` uses for `wasm-bindgen`/`wasm-pack`.
pub fn is_wasm_project(dir: &Path) -> bool {
    go_files(dir).iter().any(|f| {
        std::fs::read_to_string(f)
            .map(|s| s.contains("\"syscall/js\""))
            .unwrap_or(false)
    })
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`)
/// for `source_dir`, fail-fast (each step only runs if the previous one
/// succeeded; `paws_dagger::core` aborts the whole pipeline on the first
/// non-zero exit) — `./...` matches every package in the module, the same
/// "build/test everything" scope `cargo build`/`cargo test` give for free
/// in `paws-rust`.
///
/// The native sequence is `go build ./...`, `go vet ./...`, `go test
/// ./...`. When `is_wasm` is set (see [`is_wasm_project`]), `GOOS=js`/
/// `GOARCH=wasm` are set on the container *before* `vet`/`build` run —
/// Go's build-constraint system uses those to decide which files are even
/// included, so a `syscall/js`-guarded file is invisible without them —
/// and `go test` is skipped (see this module's doc comment on why).
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

    if is_wasm {
        args.extend([
            "with-env-variable".into(),
            "--name=GOOS".into(),
            "--value=js".into(),
        ]);
        args.extend([
            "with-env-variable".into(),
            "--name=GOARCH".into(),
            "--value=wasm".into(),
        ]);
    }

    let mut push_exec = |command_args: &[&str]| {
        args.push("with-exec".into());
        args.push(format!("--args={}", command_args.join(",")));
    };

    if is_wasm {
        push_exec(&["go", "vet", "./..."]);
        push_exec(&["go", "build", "-o", "app.wasm", "./..."]);
    } else {
        push_exec(&["go", "build", "./..."]);
        push_exec(&["go", "vet", "./..."]);
        push_exec(&["go", "test", "./..."]);
    }

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
        let args = dagger_pipeline_args("/host/src", false);
        assert_eq!(args[0], "container");
        assert_eq!(args[1], "from");
        assert_eq!(args[2], "--address=golang:1-bookworm");
    }

    #[test]
    fn pipeline_runs_the_full_build_vet_test_sequence_in_order() {
        let args = dagger_pipeline_args("/host/src", false);
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

    #[test]
    fn wasm_pipeline_sets_goos_goarch_builds_to_a_wasm_file_and_skips_go_test() {
        let args = dagger_pipeline_args("/host/src", true);
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
                "with-env-variable".to_string(),
                "--name=GOOS".to_string(),
                "--value=js".to_string(),
                "with-env-variable".to_string(),
                "--name=GOARCH".to_string(),
                "--value=wasm".to_string(),
                "with-exec".to_string(),
                "--args=go,vet,./...".to_string(),
                "with-exec".to_string(),
                "--args=go,build,-o,app.wasm,./...".to_string(),
                "stdout".to_string(),
            ]
        );
    }

    #[test]
    fn detects_wasm_project_from_a_syscall_js_import() {
        let dir = temp_dir("wasm-detect");
        fs::write(dir.join("go.mod"), "module example.com/x\n\ngo 1.23\n").unwrap();
        fs::write(
            dir.join("main.go"),
            "package main\n\nimport \"fmt\"\n\nfunc main() { fmt.Println(\"hi\") }\n",
        )
        .unwrap();
        assert!(
            !is_wasm_project(&dir),
            "a plain program with no syscall/js import isn't a wasm project"
        );

        fs::write(
            dir.join("main.go"),
            "package main\n\nimport \"syscall/js\"\n\nfunc main() { js.Global() }\n",
        )
        .unwrap();
        assert!(is_wasm_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wasm_detection_skips_vendor_and_hidden_directories() {
        let dir = temp_dir("wasm-vendor");
        fs::create_dir_all(dir.join("vendor/pkg")).unwrap();
        fs::write(
            dir.join("vendor/pkg/dep.go"),
            "package pkg\n\nimport \"syscall/js\"\n",
        )
        .unwrap();
        assert!(
            !is_wasm_project(&dir),
            "a syscall/js import inside vendor/ must not count as this module's own signal"
        );
        fs::remove_dir_all(&dir).unwrap();
    }
}
