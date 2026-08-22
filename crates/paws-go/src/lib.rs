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
//!
//! [`cross_dagger_pipeline_args`] generalizes that wasm insight into a real
//! multi-platform matrix: Go's cross-compilation story needs no cross
//! linker or extra toolchain component the way most other compiled
//! languages do (confirmed for real against `golang:1-bookworm` — plain
//! `GOOS`/`GOARCH` env vars are enough), so building for N targets in one
//! pipeline is just N `with-env-variable`×2/`with-exec`×2 groups, ending in
//! a single `directory`/`export` of the whole `dist/` folder rather than
//! `paws-release`'s one-`file`-per-invocation pattern (`crates/paws-release`)
//! — verified for real that `dagger core`'s `directory ... export` chain
//! exports a populated build directory correctly, not just single files.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

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

/// One cross-compile target, e.g. `linux/amd64`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub goos: String,
    pub goarch: String,
}

impl Target {
    /// Parses a `"<GOOS>/<GOARCH>"` spec, e.g. `"darwin/arm64"`.
    pub fn parse(spec: &str) -> Result<Self> {
        let (goos, goarch) = spec.split_once('/').ok_or_else(|| {
            anyhow::anyhow!(
                "invalid target {spec:?}, expected \"<GOOS>/<GOARCH>\" (e.g. \"linux/amd64\")"
            )
        })?;
        if goos.is_empty() || goarch.is_empty() {
            anyhow::bail!(
                "invalid target {spec:?}, expected \"<GOOS>/<GOARCH>\" (e.g. \"linux/amd64\")"
            );
        }
        Ok(Target {
            goos: goos.to_string(),
            goarch: goarch.to_string(),
        })
    }

    fn binary_suffix(&self) -> &'static str {
        if self.goos == "windows" { ".exe" } else { "" }
    }
}

/// Reads the short module name (the last path segment of `go.mod`'s
/// `module` directive) used to name each cross-compiled binary — e.g.
/// module `github.com/mbround18/paws/examples/go-fixture` names its
/// binaries `go-fixture-<goos>-<goarch>`.
pub fn module_name(dir: &Path) -> Result<String> {
    let contents = std::fs::read_to_string(dir.join("go.mod"))
        .with_context(|| format!("failed to read go.mod in {}", dir.display()))?;
    let module_path = contents
        .lines()
        .find_map(|line| line.strip_prefix("module "))
        .ok_or_else(|| anyhow::anyhow!("go.mod in {} has no `module` directive", dir.display()))?
        .trim();
    Ok(module_path
        .rsplit('/')
        .next()
        .unwrap_or(module_path)
        .to_string())
}

/// Builds the `dagger core <chain>` argument list for cross-compiling
/// `source_dir`'s Go module to every target in `targets`, exporting the
/// resulting binaries (named `<module>-<goos>-<goarch>[.exe]`) into
/// `host_dist_dir`, an absolute host path. Each target's `GOOS`/`GOARCH`
/// are set immediately before its own `go vet`/`go build -o
/// dist/<name>` pair; `go test` is skipped for every target, native
/// included — once multiple targets are being produced there's no single
/// "the" native one to special-case, and none of the resulting binaries
/// can run inside this build container regardless.
pub fn cross_dagger_pipeline_args(
    source_dir: &str,
    module: &str,
    targets: &[Target],
    host_dist_dir: &str,
) -> Vec<String> {
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

    for target in targets {
        args.extend([
            "with-env-variable".into(),
            "--name=GOOS".into(),
            format!("--value={}", target.goos),
        ]);
        args.extend([
            "with-env-variable".into(),
            "--name=GOARCH".into(),
            format!("--value={}", target.goarch),
        ]);
        let out_path = format!(
            "dist/{module}-{}-{}{}",
            target.goos,
            target.goarch,
            target.binary_suffix()
        );
        args.extend(["with-exec".into(), "--args=go,vet,./...".into()]);
        args.extend([
            "with-exec".into(),
            format!("--args=go,build,-o,{out_path},./..."),
        ]);
    }

    args.extend([
        "directory".into(),
        "--path=dist".into(),
        "export".into(),
        format!("--path={host_dist_dir}"),
    ]);
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

    #[test]
    fn target_parses_a_valid_goos_goarch_spec() {
        let target = Target::parse("linux/amd64").unwrap();
        assert_eq!(target.goos, "linux");
        assert_eq!(target.goarch, "amd64");
    }

    #[test]
    fn target_rejects_a_spec_with_no_slash() {
        assert!(Target::parse("linux-amd64").is_err());
    }

    #[test]
    fn target_rejects_an_empty_goos_or_goarch() {
        assert!(Target::parse("/amd64").is_err());
        assert!(Target::parse("linux/").is_err());
    }

    #[test]
    fn windows_targets_get_an_exe_suffix_others_dont() {
        assert_eq!(
            Target::parse("windows/amd64").unwrap().binary_suffix(),
            ".exe"
        );
        assert_eq!(Target::parse("linux/amd64").unwrap().binary_suffix(), "");
        assert_eq!(Target::parse("darwin/arm64").unwrap().binary_suffix(), "");
    }

    #[test]
    fn module_name_reads_the_last_path_segment_of_go_mod() {
        let dir = temp_dir("module-name");
        fs::write(
            dir.join("go.mod"),
            "module github.com/mbround18/paws/examples/go-fixture\n\ngo 1.23\n",
        )
        .unwrap();
        assert_eq!(module_name(&dir).unwrap(), "go-fixture");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn module_name_errors_without_a_go_mod() {
        let dir = temp_dir("module-name-missing");
        assert!(module_name(&dir).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cross_pipeline_sets_env_and_builds_each_target_then_exports_dist() {
        let targets = vec![
            Target::parse("linux/amd64").unwrap(),
            Target::parse("windows/arm64").unwrap(),
        ];
        let args = cross_dagger_pipeline_args("/host/src", "app", &targets, "/host/dist");
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
                "--value=linux".to_string(),
                "with-env-variable".to_string(),
                "--name=GOARCH".to_string(),
                "--value=amd64".to_string(),
                "with-exec".to_string(),
                "--args=go,vet,./...".to_string(),
                "with-exec".to_string(),
                "--args=go,build,-o,dist/app-linux-amd64,./...".to_string(),
                "with-env-variable".to_string(),
                "--name=GOOS".to_string(),
                "--value=windows".to_string(),
                "with-env-variable".to_string(),
                "--name=GOARCH".to_string(),
                "--value=arm64".to_string(),
                "with-exec".to_string(),
                "--args=go,vet,./...".to_string(),
                "with-exec".to_string(),
                "--args=go,build,-o,dist/app-windows-arm64.exe,./...".to_string(),
                "directory".to_string(),
                "--path=dist".to_string(),
                "export".to_string(),
                "--path=/host/dist".to_string(),
            ]
        );
    }
}
