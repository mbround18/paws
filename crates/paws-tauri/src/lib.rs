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

use paws_core::{Base, ContainerOptions, container::SOURCE_ROOT};
use std::path::{Path, PathBuf};

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
/// general-purpose tool run anywhere. Embedding the text and writing it into
/// the build context (see [`paws_core::Base::Dockerfile`]) makes this
/// correct regardless of where `paws` is invoked from.
pub const TAURI_LINUX_DOCKERFILE: &str = include_str!("../../../builders/tauri-linux/Dockerfile");

/// The Tauri Android builder Dockerfile (JDK + Android SDK/NDK + Rust
/// Android targets + Node), embedded the same way and for the same reason
/// as [`TAURI_LINUX_DOCKERFILE`]. There is no Android equivalent of the
/// iOS problem: the SDK/NDK are plain redistributable downloads and the
/// whole toolchain runs on Linux, unlike Xcode/`xcodebuild` (see
/// `builders/tauri-android/Dockerfile`'s header comment, and the iOS note
/// in `docs/ROADMAP.md`).
pub const TAURI_ANDROID_DOCKERFILE: &str =
    include_str!("../../../builders/tauri-android/Dockerfile");

/// Where a Tauri app sits relative to what has to be sent to the container.
///
/// A standalone app (its `src-tauri` crate is its own Cargo root) sends
/// itself. An app inside a Cargo workspace — `ui/src-tauri` a member of the
/// repo-root workspace, depending on sibling crates — has to send the
/// workspace root instead, or the Rust build finds neither `Cargo.lock` nor
/// the crates it depends on; the Tauri CLI then runs from the app's
/// subdirectory, where its `package.json` and `tauri.conf.json` live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    /// Host directory sent to the container, mounted at
    /// [`paws_core::container::SOURCE_ROOT`].
    pub context_dir: PathBuf,
    /// The app directory, relative to `context_dir` (empty when they are
    /// the same directory).
    pub app_subdir: PathBuf,
}

impl Layout {
    /// `/src` or `/src/<app_subdir>`: where the package manager runs.
    pub fn workdir(&self) -> String {
        if self.app_subdir.as_os_str().is_empty() {
            SOURCE_ROOT.to_string()
        } else {
            format!("{SOURCE_ROOT}/{}", self.app_subdir.to_string_lossy())
        }
    }
}

/// Finds the directory to send for the Tauri app at `app_dir` (which must
/// be absolute and canonical): the nearest ancestor of its `src-tauri`
/// crate whose `Cargo.toml` declares a `[workspace]`, the way `cargo` itself
/// locates the workspace root — or `app_dir` when that crate is its own
/// root, or no workspace manifest exists above it.
pub fn layout(app_dir: &Path) -> Result<Layout> {
    let crate_dir = app_dir.join("src-tauri");
    let root = workspace_root(&crate_dir)?.filter(|root| root != &crate_dir);
    match root {
        Some(root) if root != app_dir => {
            let app_subdir = app_dir
                .strip_prefix(&root)
                .with_context(|| {
                    format!(
                        "{} is the Cargo workspace root but not an ancestor of {}",
                        root.display(),
                        app_dir.display()
                    )
                })?
                .to_path_buf();
            Ok(Layout {
                context_dir: root,
                app_subdir,
            })
        }
        _ => Ok(Layout {
            context_dir: app_dir.to_path_buf(),
            app_subdir: PathBuf::new(),
        }),
    }
}

/// The nearest directory at or above `dir` whose `Cargo.toml` has a
/// `[workspace]` table.
fn workspace_root(dir: &Path) -> Result<Option<PathBuf>> {
    for candidate in dir.ancestors() {
        let manifest = candidate.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&manifest)
            .with_context(|| format!("failed to read {}", manifest.display()))?;
        let parsed: toml::Table = toml::from_str(&text)
            .with_context(|| format!("failed to parse {}", manifest.display()))?;
        if parsed.contains_key("workspace") {
            return Ok(Some(candidate.to_path_buf()));
        }
    }
    Ok(None)
}

fn pipeline_args(
    project: &NodeProject,
    layout: &Layout,
    container: &ContainerOptions,
    dockerfile: &str,
    tauri_subcommand: &[&str],
) -> Result<Vec<String>> {
    let pm = project.package_manager;
    let mut pipeline = container
        .open(
            &layout.context_dir.to_string_lossy(),
            &Base::Dockerfile(dockerfile),
        )?
        .workdir(&layout.workdir());

    if let Some(setup) = pm.setup_args() {
        pipeline = pipeline.exec(setup);
    }
    pipeline = pipeline.exec(pm.install_args(project.has_lockfile));
    // `<pm> run tauri [android] build` — the "tauri" script (aliasing
    // `@tauri-apps/cli`, which every `create-tauri-app` scaffold defines)
    // takes the subcommand as positional arguments, not a second script name.
    let mut tauri_build = pm.run_script_args("tauri");
    tauri_build.extend(tauri_subcommand.iter().map(ToString::to_string));

    // Cheap checks first: a failing unit test or lint should cost a minute,
    // not the 20-minute Rust release build it used to follow.
    let pipeline = pipeline
        .exec_if(project.has_lint_script, pm.run_script_args("lint"))
        .exec_if(project.has_test_script, pm.run_script_args("test"))
        .exec(tauri_build);
    Ok(container.finish(pipeline))
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`)
/// that builds the Tauri Linux builder ([`TAURI_LINUX_DOCKERFILE`], plus
/// `container`'s packages and setup — Dagger's own `BuildKit` layer caching
/// means the slow system-dependency install only actually runs once per
/// unchanged Dockerfile, not on every `paws ci` invocation) over the
/// filtered `layout.context_dir`, then installs dependencies and runs
/// `<package manager> run tauri build` for `project` from the app's own
/// directory — which itself runs the frontend build (via `tauri.conf.json`'s
/// `beforeBuildCommand`) before compiling the Rust shell, so this crate
/// never has to sequence that itself. Runs `lint`/`test` first, only if
/// the project actually defines them — unlike `paws-node`'s plain pipeline,
/// `build`+`test` aren't required here (a fresh Tauri scaffold has neither;
/// `tauri build` is the meaningful step). With `container.export`, the chain
/// ends by copying that directory (the bundles, say) back to the host.
pub fn dagger_pipeline_args(
    project: &NodeProject,
    layout: &Layout,
    container: &ContainerOptions,
) -> Result<Vec<String>> {
    pipeline_args(
        project,
        layout,
        container,
        TAURI_LINUX_DOCKERFILE,
        &["build"],
    )
}

/// Same as [`dagger_pipeline_args`], but builds against the Tauri Android
/// builder ([`TAURI_ANDROID_DOCKERFILE`]) and runs `<package manager> run
/// tauri android build` instead. Assumes the target repo has already run
/// `tauri android init` (`src-tauri/gen/android` committed) — `paws` builds
/// what's there, it doesn't scaffold mobile projects itself.
pub fn android_dagger_pipeline_args(
    project: &NodeProject,
    layout: &Layout,
    container: &ContainerOptions,
) -> Result<Vec<String>> {
    pipeline_args(
        project,
        layout,
        container,
        TAURI_ANDROID_DOCKERFILE,
        &["android", "build"],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use paws_node::{Framework, PackageManager};
    use std::fs;

    /// A scratch dir that already looks like a Tauri project — every test
    /// here needs `src-tauri/` to exist before it can plant a config in it.
    fn temp_dir(name: &str) -> PathBuf {
        let dir = paws_core::test_support::scratch_dir("tauri", name);
        fs::create_dir_all(dir.join("src-tauri")).unwrap();
        dir
    }

    fn standalone() -> Layout {
        Layout {
            context_dir: PathBuf::from("/host/src"),
            app_subdir: PathBuf::new(),
        }
    }

    #[test]
    fn detects_tauri_project_from_config_file() {
        let dir = temp_dir("detect");
        assert!(
            !is_tauri_project(&dir),
            "should not detect before tauri.conf.json exists"
        );
        fs::write(dir.join("src-tauri").join("tauri.conf.json"), "{}").unwrap();
        assert!(is_tauri_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_standalone_app_is_its_own_context() {
        let dir = temp_dir("layout-standalone");
        fs::write(
            dir.join("src-tauri/Cargo.toml"),
            "[package]\nname = \"app\"\n\n[workspace]\n",
        )
        .unwrap();
        let dir = dir.canonicalize().unwrap();
        let layout = layout(&dir).unwrap();
        assert_eq!(layout.context_dir, dir);
        assert_eq!(layout.app_subdir, PathBuf::new());
        assert_eq!(layout.workdir(), "/src");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_app_without_any_workspace_manifest_is_also_its_own_context() {
        let dir = temp_dir("layout-no-workspace");
        fs::write(
            dir.join("src-tauri/Cargo.toml"),
            "[package]\nname = \"app\"\n",
        )
        .unwrap();
        let dir = dir.canonicalize().unwrap();
        let layout = layout(&dir).unwrap();
        assert_eq!(layout.context_dir, dir);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_workspace_member_sends_the_workspace_root_and_runs_from_the_app_dir() {
        let root = paws_core::test_support::scratch_dir("tauri", "layout-workspace");
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\".\", \"ui/src-tauri\"]\n\n[package]\nname = \"repo\"\n",
        )
        .unwrap();
        let app = root.join("ui");
        fs::create_dir_all(app.join("src-tauri")).unwrap();
        fs::write(
            app.join("src-tauri/Cargo.toml"),
            "[package]\nname = \"app\"\n",
        )
        .unwrap();
        let root = root.canonicalize().unwrap();
        let layout = layout(&root.join("ui")).unwrap();
        assert_eq!(layout.context_dir, root);
        assert_eq!(layout.app_subdir, PathBuf::from("ui"));
        assert_eq!(layout.workdir(), "/src/ui");
        fs::remove_dir_all(&root).unwrap();
    }

    fn project() -> NodeProject {
        NodeProject {
            package_manager: PackageManager::Npm,
            framework: Framework::Vite,
            has_build_script: true,
            has_test_script: false,
            has_lint_script: false,
            has_lockfile: true,
            has_playwright: false,
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
    fn pipeline_builds_the_embedded_builder_over_the_filtered_context() {
        let args =
            dagger_pipeline_args(&project(), &standalone(), &ContainerOptions::default()).unwrap();
        assert_eq!(
            &args[..4],
            &["host", "directory", "--path=/host/src", "--gitignore"]
        );
        let dockerfile = args
            .iter()
            .find(|a| a.starts_with("--contents=") && a.contains("FROM "))
            .unwrap();
        assert!(dockerfile.contains("libwebkit2gtk-4.1-dev"));
        assert!(dockerfile.ends_with("WORKDIR /src\nCOPY . /src"));
        assert!(args.contains(&"docker-build".to_string()));
        assert!(
            args.iter()
                .any(|a| a.starts_with("--build-args=BUILDER_VERSION="))
        );
    }

    #[test]
    fn pipeline_runs_tauri_build_via_the_detected_package_manager() {
        let args =
            dagger_pipeline_args(&project(), &standalone(), &ContainerOptions::default()).unwrap();
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
        let args =
            dagger_pipeline_args(&with_both, &standalone(), &ContainerOptions::default()).unwrap();
        assert!(args.contains(&"--args=npm,run,test".to_string()));
        assert!(args.contains(&"--args=npm,run,lint".to_string()));
    }

    #[test]
    fn a_workspace_layout_runs_from_the_app_subdir_and_can_export_the_bundles() {
        let layout = Layout {
            context_dir: PathBuf::from("/host/repo"),
            app_subdir: PathBuf::from("ui"),
        };
        let container = ContainerOptions {
            export: Some(paws_core::Export {
                path: "/src/target/release/bundle".into(),
                destination: "/host/repo/dist".into(),
            }),
            ..Default::default()
        };
        let args = dagger_pipeline_args(&project(), &layout, &container).unwrap();
        assert_eq!(args[2], "--path=/host/repo");
        let position = |needle: &str| args.iter().position(|a| a == needle).unwrap();
        assert!(position("--path=/src/ui") < position("--args=npm,ci"));
        assert!(position("--args=npm,run,tauri,build") < position("export"));
        if let Some(lint) = args.iter().position(|a| a == "--args=npm,run,lint") {
            assert!(
                lint < position("--args=npm,run,tauri,build"),
                "lint runs before the slow build"
            );
        }
        assert_eq!(args.last().unwrap(), "--path=/host/repo/dist");
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
    fn android_pipeline_runs_tauri_android_build() {
        let args =
            android_dagger_pipeline_args(&project(), &standalone(), &ContainerOptions::default())
                .unwrap();
        let dockerfile = args
            .iter()
            .find(|a| a.starts_with("--contents=") && a.contains("FROM "))
            .unwrap();
        assert!(dockerfile.contains("ANDROID_NDK_VERSION"));
        assert!(args.contains(&"--args=npm,run,tauri,android,build".to_string()));
        assert!(!args.iter().any(|a| a == "--args=npm,run,tauri,build"));
    }
}
