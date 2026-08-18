//! Tauri project detection and CI pipeline construction. Tauri is the first
//! "composite" stack `paws` supports: unlike plain Rust or plain Node, a
//! Tauri app has a real ordering dependency (the frontend must build before
//! the Rust shell bundles it) rather than two independent toolchains that
//! can just be provisioned concurrently (spec.md FR-016 already carves this
//! distinction out). `paws` doesn't reimplement that sequencing itself —
//! Tauri's own CLI already does it via `tauri.conf.json`'s
//! `beforeBuildCommand`, so this crate's job is detecting a Tauri project
//! and invoking that CLI correctly, with both toolchains available in one
//! container.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use paws_node::NodeProject;

/// A Tauri project is a Node project (package.json at the root) with a
/// `src-tauri/tauri.conf.json` — the config file the Tauri CLI itself
/// requires, so its presence is as reliable a signal as Tauri projects get.
pub fn is_tauri_project(dir: &Path) -> bool {
    dir.join("src-tauri").join("tauri.conf.json").is_file()
}

/// The Tauri Linux builder Dockerfile (Rust + Node + the GTK/WebKit
/// libraries Tauri's Linux backend needs), embedded at compile time from
/// `builders/tauri-linux/Dockerfile`. `paws ci` runs from inside whatever
/// *target* repo it's checking, not from inside `paws`'s own source tree —
/// unlike `paws-release`'s builders (which only ever run from within
/// `paws`'s own repo, building `paws` itself), a repo-relative
/// `builders/tauri-linux` path would silently resolve against the wrong
/// directory the moment `paws` is used the way it's meant to be: as a
/// general-purpose tool run anywhere. Embedding + materializing to a temp
/// dir (see [`write_builder_dockerfile`]) makes this correct regardless of
/// where `paws` is invoked from.
const TAURI_LINUX_DOCKERFILE: &str = include_str!("../../../builders/tauri-linux/Dockerfile");

/// The Tauri Android builder Dockerfile (JDK + Android SDK/NDK + Rust
/// Android targets + Node), embedded the same way and for the same reason
/// as [`TAURI_LINUX_DOCKERFILE`]. There is no Android equivalent of the
/// iOS problem: the SDK/NDK are plain redistributable downloads and the
/// whole toolchain runs on Linux, unlike Xcode/`xcodebuild` (see
/// `builders/tauri-android/Dockerfile`'s header comment, and the iOS note
/// in `docs/ROADMAP.md`).
const TAURI_ANDROID_DOCKERFILE: &str = include_str!("../../../builders/tauri-android/Dockerfile");

fn write_dockerfile(name: &str, contents: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("paws-builders").join(name);
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create temp dir for the {name} builder Dockerfile"))?;
    std::fs::write(dir.join("Dockerfile"), contents)
        .with_context(|| format!("failed to write the {name} builder Dockerfile"))?;
    Ok(dir)
}

/// Writes the embedded Tauri Linux builder Dockerfile to a temp directory
/// and returns that directory's path, suitable for `dagger_pipeline_args`'s
/// `builder_dir` argument.
pub fn write_builder_dockerfile() -> Result<PathBuf> {
    write_dockerfile("tauri-linux", TAURI_LINUX_DOCKERFILE)
}

/// Writes the embedded Tauri Android builder Dockerfile to a temp directory
/// and returns that directory's path, suitable for
/// `android_dagger_pipeline_args`'s `builder_dir` argument.
pub fn write_android_builder_dockerfile() -> Result<PathBuf> {
    write_dockerfile("tauri-android", TAURI_ANDROID_DOCKERFILE)
}

fn pipeline_args(
    project: &NodeProject,
    source_dir: &str,
    builder_dir: &str,
    tauri_subcommand: &[&str],
) -> Vec<String> {
    let pm = project.package_manager;
    let created_unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let build_args = format!("BUILDER_VERSION=dev,BUILDER_REVISION=unknown,BUILDER_CREATED={created_unix}");

    let mut args: Vec<String> = vec![
        "host".into(),
        "directory".into(),
        format!("--path={builder_dir}"),
        "docker-build".into(),
        format!("--build-args={build_args}"),
        "with-mounted-directory".into(),
        "--path=/src".into(),
        format!("--source={source_dir}"),
        "with-workdir".into(),
        "--path=/src".into(),
    ];

    let mut push_exec = |command_args: Vec<String>| {
        args.push("with-exec".into());
        args.push(format!("--args={}", command_args.join(",")));
    };

    if let Some(setup) = pm.setup_args() {
        push_exec(setup.iter().map(|s| s.to_string()).collect());
    }
    push_exec(pm.install_args(project.has_lockfile).iter().map(|s| s.to_string()).collect());
    // `<pm> run tauri [android] build` — the "tauri" script (aliasing
    // `@tauri-apps/cli`, which every `create-tauri-app` scaffold defines)
    // takes the subcommand as positional arguments, not a second script name.
    let mut tauri_build = pm.run_script_args("tauri");
    tauri_build.extend(tauri_subcommand.iter().map(|s| s.to_string()));
    push_exec(tauri_build);
    if project.has_test_script {
        push_exec(pm.run_script_args("test"));
    }
    if project.has_lint_script {
        push_exec(pm.run_script_args("lint"));
    }

    args.push("stdout".into());
    args
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`)
/// that builds the Tauri Linux builder from `builder_dir` (see
/// [`write_builder_dockerfile`] — Dagger's own BuildKit layer caching means
/// the slow system-dependency install only actually runs once per unchanged
/// Dockerfile, not on every `paws ci` invocation), then installs
/// dependencies and runs `<package manager> run tauri build` for `project`
/// — which itself runs the frontend build (via `tauri.conf.json`'s
/// `beforeBuildCommand`) before compiling the Rust shell, so this crate
/// never has to sequence that itself. Runs `test`/`lint` afterward only if
/// the project actually defines them — unlike `paws-node`'s plain pipeline,
/// `build`+`test` aren't required here (a fresh Tauri scaffold has neither;
/// `tauri build` is the meaningful step).
pub fn dagger_pipeline_args(project: &NodeProject, source_dir: &str, builder_dir: &str) -> Vec<String> {
    pipeline_args(project, source_dir, builder_dir, &["build"])
}

/// Same as [`dagger_pipeline_args`], but builds against the Tauri Android
/// builder (see [`write_android_builder_dockerfile`]) and runs
/// `<package manager> run tauri android build` instead. Assumes the target
/// repo has already run `tauri android init` (`src-tauri/gen/android`
/// committed) — `paws` builds what's there, it doesn't scaffold mobile
/// projects itself.
pub fn android_dagger_pipeline_args(project: &NodeProject, source_dir: &str, builder_dir: &str) -> Vec<String> {
    pipeline_args(project, source_dir, builder_dir, &["android", "build"])
}

#[cfg(test)]
mod tests {
    use super::*;
    use paws_node::{Framework, PackageManager};
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("paws-tauri-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src-tauri")).unwrap();
        dir
    }

    #[test]
    fn detects_tauri_project_from_config_file() {
        let dir = temp_dir("detect");
        assert!(!is_tauri_project(&dir), "should not detect before tauri.conf.json exists");
        fs::write(dir.join("src-tauri").join("tauri.conf.json"), "{}").unwrap();
        assert!(is_tauri_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    fn project() -> NodeProject {
        NodeProject {
            package_manager: PackageManager::Npm,
            framework: Framework::Vite,
            has_build_script: true,
            has_test_script: false,
            has_lint_script: false,
            has_lockfile: true,
        }
    }

    #[test]
    fn tauri_linux_builder_dockerfile_exists() {
        let dockerfile = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("builders/tauri-linux")
            .join("Dockerfile");
        assert!(dockerfile.is_file(), "missing {dockerfile:?}");
    }

    #[test]
    fn write_builder_dockerfile_materializes_the_embedded_dockerfile() {
        let dir = write_builder_dockerfile().unwrap();
        let contents = fs::read_to_string(dir.join("Dockerfile")).unwrap();
        assert_eq!(contents, TAURI_LINUX_DOCKERFILE);
    }

    #[test]
    fn pipeline_builds_against_the_given_builder_dir() {
        let args = dagger_pipeline_args(&project(), "/host/src", "/tmp/some-builder-dir");
        assert_eq!(args[0], "host");
        assert_eq!(args[2], "--path=/tmp/some-builder-dir");
        assert_eq!(args[3], "docker-build");
        assert!(args[4].starts_with("--build-args=BUILDER_VERSION="));
    }

    #[test]
    fn pipeline_runs_tauri_build_via_the_detected_package_manager() {
        let args = dagger_pipeline_args(&project(), "/host/src", "/tmp/some-builder-dir");
        assert!(args.contains(&"--args=npm,ci".to_string()));
        assert!(args.contains(&"--args=npm,run,tauri,build".to_string()));
        // no test/lint scripts on this fixture project -> neither should run
        assert!(!args.iter().any(|a| a == "--args=npm,run,test"));
        assert!(!args.iter().any(|a| a == "--args=npm,run,lint"));
        assert_eq!(args.last(), Some(&"stdout".to_string()));
    }

    #[test]
    fn pipeline_runs_test_and_lint_when_the_project_defines_them() {
        let mut with_both = project();
        with_both.has_test_script = true;
        with_both.has_lint_script = true;
        let args = dagger_pipeline_args(&with_both, "/host/src", "/tmp/some-builder-dir");
        assert!(args.contains(&"--args=npm,run,test".to_string()));
        assert!(args.contains(&"--args=npm,run,lint".to_string()));
    }

    #[test]
    fn tauri_android_builder_dockerfile_exists() {
        let dockerfile = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("builders/tauri-android")
            .join("Dockerfile");
        assert!(dockerfile.is_file(), "missing {dockerfile:?}");
    }

    #[test]
    fn write_android_builder_dockerfile_materializes_the_embedded_dockerfile() {
        let dir = write_android_builder_dockerfile().unwrap();
        let contents = fs::read_to_string(dir.join("Dockerfile")).unwrap();
        assert_eq!(contents, TAURI_ANDROID_DOCKERFILE);
    }

    #[test]
    fn android_pipeline_runs_tauri_android_build() {
        let args = android_dagger_pipeline_args(&project(), "/host/src", "/tmp/some-android-builder-dir");
        assert_eq!(args[2], "--path=/tmp/some-android-builder-dir");
        assert!(args.contains(&"--args=npm,run,tauri,android,build".to_string()));
        assert!(!args.iter().any(|a| a == "--args=npm,run,tauri,build"));
    }
}
