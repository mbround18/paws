//! `paws run`: a project's own commands, in a container, the same way on a
//! laptop and on a CI runner.
//!
//! Every `paws ci --toolchain` pipeline is a fixed recipe: `cargo fmt`,
//! `cargo clippy`, `cargo build`, `cargo test`, with the flags that recipe
//! chose. A real repo's CI rarely stops there. It checks generated code for
//! drift, runs a guard script, lints one crate with different flags, needs a
//! system library the base image lacks. Before this, each of those steps had
//! to live in the CI provider's YAML, which is exactly the part that cannot
//! be run locally. `paws run` is the escape hatch that keeps them inside
//! paws: an image, some Debian packages, a working directory and a list of
//! commands, run by the same `dagger` engine `paws ci` uses.
//!
//! What goes into the container is the host directory filtered on the host
//! (see [`paws_core::Pipeline::from_host_context`]): `.gitignore` applies,
//! `.git` is left out by default, so a local run sees what a fresh CI
//! checkout sees rather than a workstation's build outputs.
//!
//! This crate only *describes* the pipeline; `paws-cli-core` runs it.

use anyhow::{Context, Result, bail};
use paws_core::Pipeline;

/// Where the source directory lands inside the container.
pub const SOURCE_ROOT: &str = "/src";

/// Excluded from every run on top of `.gitignore` and `--exclude`.
///
/// `.git` is not in any `.gitignore`, changes on every commit (so it would
/// defeat step caching even when no tracked file changed), and in a git
/// worktree it is a file pointing outside the directory, so it is not usable
/// inside the container anyway.
pub const DEFAULT_EXCLUDES: &[&str] = &[".git"];

/// A named cache volume and where it is mounted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheMount {
    pub name: String,
    pub path: String,
}

/// Everything one `paws run` needs, already resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSpec {
    /// Host directory sent to the container, mounted at [`SOURCE_ROOT`].
    pub context_dir: String,
    /// Container directory the commands run in.
    pub workdir: String,
    pub image: String,
    pub apt_packages: Vec<String>,
    /// Shell commands baked into the image after the packages, each its own
    /// `RUN` layer.
    pub setup: Vec<String>,
    pub env: Vec<(String, String)>,
    pub caches: Vec<CacheMount>,
    /// Extra patterns left out of the context, beyond `.gitignore`.
    pub excludes: Vec<String>,
    /// Each command runs as its own step, in order, stopping at the first
    /// failure.
    pub commands: Vec<Vec<String>>,
}

/// The Dockerfile a run is built from: the image, then the Debian packages
/// and the `--setup` commands (before the source copy, so changing a source
/// file never reruns them), then the source.
pub fn dockerfile(image: &str, apt_packages: &[String], setup: &[String]) -> String {
    let mut lines = vec![format!("FROM {image}")];
    if !apt_packages.is_empty() {
        lines.push(format!(
            "RUN apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends {} && rm -rf /var/lib/apt/lists/*",
            apt_packages.join(" ")
        ));
    }
    lines.extend(setup.iter().map(|script| format!("RUN {script}")));
    lines.push(format!("WORKDIR {SOURCE_ROOT}"));
    lines.push(format!("COPY . {SOURCE_ROOT}"));
    lines.join("\n")
}

/// Rejects anything that is not a plain Debian package name (optionally
/// `name=version` or `name:arch`). These are spliced into a `RUN` line, so a
/// stray `;` or space would otherwise become shell.
pub fn validate_apt_package(name: &str) -> Result<()> {
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-' | ':' | '=' | '~'));
    if !valid {
        bail!("--apt {name:?} is not a Debian package name");
    }
    Ok(())
}

/// Rejects a `--setup` command that would not be one Dockerfile `RUN` line:
/// an empty one, or one spanning lines (the next line would be read as a new
/// Dockerfile instruction). Chain commands with `&&` instead.
pub fn validate_setup(script: &str) -> Result<()> {
    if script.trim().is_empty() {
        bail!("--setup is empty");
    }
    if script.contains(['\n', '\r']) {
        bail!("--setup {script:?} spans lines; join its commands with && instead");
    }
    Ok(())
}

/// The container directory for `--workdir`, a path relative to the source
/// directory. Absolute paths and `..` are refused: the point of `--workdir`
/// is "run from this part of the source", not "run somewhere else".
pub fn container_workdir(workdir: Option<&str>) -> Result<String> {
    let Some(workdir) = workdir
        .map(|w| w.trim_matches('/'))
        .filter(|w| !w.is_empty() && *w != ".")
    else {
        return Ok(SOURCE_ROOT.to_string());
    };
    if workdir.split('/').any(|part| part == "..") {
        bail!("--workdir {workdir:?} must stay inside the source directory");
    }
    Ok(format!("{SOURCE_ROOT}/{workdir}"))
}

/// Parses one `--env` entry. `NAME=value` sets a value; a bare `NAME` passes
/// the host's value through (`lookup`), and is skipped when the host has none,
/// so one invocation can forward optional knobs like `CARGO_BUILD_JOBS`
/// without every caller having to set them.
pub fn parse_env(
    entry: &str,
    lookup: impl Fn(&str) -> Option<String>,
) -> Result<Option<(String, String)>> {
    let (name, value) = match entry.split_once('=') {
        Some((name, value)) => (name, Some(value.to_string())),
        None => (entry, None),
    };
    let valid_name = !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid_name {
        bail!("--env {entry:?}: {name:?} is not an environment variable name");
    }
    Ok(value
        .or_else(|| lookup(name))
        .map(|value| (name.to_string(), value)))
}

/// Parses one `--cache` entry: `PATH` or `NAME=PATH`.
///
/// A relative `PATH` is relative to `workdir` (so `--cache target` in a Rust
/// repo caches its build directory). Without a `NAME`, the volume is named
/// after `scope` (the source directory's name) and the path, so two repos
/// don't share a `target/` but every run in one repo does.
pub fn parse_cache(entry: &str, scope: &str, workdir: &str) -> Result<CacheMount> {
    let (name, path) = match entry.split_once('=') {
        Some((name, path)) => (Some(name), path),
        None => (None, entry),
    };
    if path.is_empty() {
        bail!("--cache {entry:?} has no path");
    }
    let path = if path.starts_with('/') {
        path.trim_end_matches('/').to_string()
    } else {
        format!(
            "{}/{}",
            workdir.trim_end_matches('/'),
            path.trim_end_matches('/')
        )
    };
    let name = name.map_or_else(
        || format!("paws-run-{}{}", slug(scope), slug(&path)),
        ToString::to_string,
    );
    if name.is_empty() {
        bail!("--cache {entry:?} has an empty name");
    }
    Ok(CacheMount { name, path })
}

/// Lowercase alphanumerics, with every other run of characters as one `-`.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out
}

/// Builds the `dagger core <chain>` argument list for `spec`: the filtered
/// source built into the image, then caches, environment and working
/// directory, then each command as its own `with-exec`.
pub fn dagger_pipeline_args(spec: &RunSpec) -> Result<Vec<String>> {
    if spec.commands.is_empty() {
        bail!("nothing to run: pass --step \"<shell>\" and/or a command after --");
    }
    for package in &spec.apt_packages {
        validate_apt_package(package)?;
    }
    for script in &spec.setup {
        validate_setup(script)?;
    }

    let mut excludes: Vec<String> = DEFAULT_EXCLUDES.iter().map(ToString::to_string).collect();
    excludes.extend(spec.excludes.iter().cloned());

    let mut pipeline = Pipeline::from_host_context(
        &spec.context_dir,
        &excludes,
        &dockerfile(&spec.image, &spec.apt_packages, &spec.setup),
    );
    for cache in &spec.caches {
        pipeline = pipeline.mount_cache(&cache.path, &cache.name);
    }
    for (name, value) in &spec.env {
        pipeline = pipeline.env(name, value);
    }
    pipeline = pipeline.workdir(&spec.workdir);
    for command in &spec.commands {
        pipeline = pipeline.exec(command);
    }
    Ok(pipeline.stdout())
}

/// A `--step` script as a command: `sh -c <script>`.
pub fn shell_step(script: &str) -> Vec<String> {
    vec!["sh".into(), "-c".into(), script.into()]
}

/// The name `--cache` volumes are scoped by: the source directory's own name.
pub fn cache_scope(context_dir: &std::path::Path) -> Result<String> {
    context_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .context("the source directory has no name to scope --cache volumes by")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> RunSpec {
        RunSpec {
            context_dir: "/host/repo".into(),
            workdir: "/src".into(),
            image: "rust:1-bookworm".into(),
            apt_packages: vec![],
            setup: vec![],
            env: vec![],
            caches: vec![],
            excludes: vec![],
            commands: vec![vec!["cargo".into(), "test".into()]],
        }
    }

    #[test]
    fn the_dockerfile_installs_packages_before_copying_the_source() {
        let file = dockerfile(
            "rust:1-bookworm",
            &["libwebkit2gtk-4.1-dev".into(), "cmake".into()],
            &["rustup component add clippy rustfmt".into()],
        );
        let lines: Vec<&str> = file.lines().collect();
        assert_eq!(lines[0], "FROM rust:1-bookworm");
        assert!(lines[1].contains("apt-get install"));
        assert!(lines[1].contains("libwebkit2gtk-4.1-dev cmake"));
        assert_eq!(lines[2], "RUN rustup component add clippy rustfmt");
        assert_eq!(lines[3], "WORKDIR /src");
        assert_eq!(lines[4], "COPY . /src");
    }

    #[test]
    fn no_packages_means_no_apt_layer() {
        assert!(!dockerfile("alpine:3", &[], &[]).contains("apt-get"));
    }

    #[test]
    fn a_multi_line_setup_is_refused_before_anything_runs() {
        validate_setup("apt-get update && apt-get install -y x").unwrap();
        let mut spec = spec();
        spec.setup = vec!["echo a\nFROM evil".into()];
        assert!(dagger_pipeline_args(&spec).is_err());
        spec.setup = vec!["  ".into()];
        assert!(dagger_pipeline_args(&spec).is_err());
    }

    #[test]
    fn apt_package_names_cannot_smuggle_shell() {
        validate_apt_package("libwebkit2gtk-4.1-dev").unwrap();
        validate_apt_package("g++").unwrap();
        validate_apt_package("nodejs=24.1.0-1nodesource1").unwrap();
        assert!(validate_apt_package("curl; rm -rf /").is_err());
        assert!(validate_apt_package("a b").is_err());
        assert!(validate_apt_package("").is_err());
    }

    #[test]
    fn workdir_is_relative_to_the_source_and_cannot_escape_it() {
        assert_eq!(container_workdir(None).unwrap(), "/src");
        assert_eq!(container_workdir(Some(".")).unwrap(), "/src");
        assert_eq!(container_workdir(Some("ui")).unwrap(), "/src/ui");
        assert_eq!(
            container_workdir(Some("ui/src-tauri/")).unwrap(),
            "/src/ui/src-tauri"
        );
        assert!(container_workdir(Some("../elsewhere")).is_err());
        assert!(container_workdir(Some("ui/../../x")).is_err());
    }

    #[test]
    fn env_sets_a_value_or_passes_the_host_value_through() {
        let host = |name: &str| (name == "CARGO_BUILD_JOBS").then(|| "4".to_string());
        assert_eq!(
            parse_env("CI=true", host).unwrap(),
            Some(("CI".into(), "true".into()))
        );
        assert_eq!(
            parse_env("CARGO_BUILD_JOBS", host).unwrap(),
            Some(("CARGO_BUILD_JOBS".into(), "4".into()))
        );
        assert_eq!(parse_env("UNSET_ON_HOST", host).unwrap(), None);
        assert_eq!(
            parse_env("FLAGS=-D warnings=x", host).unwrap(),
            Some(("FLAGS".into(), "-D warnings=x".into()))
        );
        assert!(parse_env("1BAD=x", host).is_err());
        assert!(parse_env("=x", host).is_err());
    }

    #[test]
    fn a_cache_is_named_after_the_repo_and_path_unless_named_explicitly() {
        let cache = parse_cache("target", "fathom", "/src").unwrap();
        assert_eq!(cache.path, "/src/target");
        assert_eq!(cache.name, "paws-run-fathom-src-target");

        let cache = parse_cache("/usr/local/cargo/registry/", "My Repo", "/src/ui").unwrap();
        assert_eq!(cache.path, "/usr/local/cargo/registry");
        assert_eq!(cache.name, "paws-run-my-repo-usr-local-cargo-registry");

        let cache = parse_cache("cargo-registry=/usr/local/cargo/registry", "x", "/src").unwrap();
        assert_eq!(cache.name, "cargo-registry");

        let relative = parse_cache("node_modules", "x", "/src/ui").unwrap();
        assert_eq!(relative.path, "/src/ui/node_modules");

        assert!(parse_cache("", "x", "/src").is_err());
        assert!(parse_cache("=/path", "x", "/src").is_err());
    }

    #[test]
    fn the_pipeline_opens_on_the_filtered_source_and_always_leaves_out_git() {
        let mut spec = spec();
        spec.excludes = vec!["ui/node_modules".into()];
        let args = dagger_pipeline_args(&spec).unwrap();
        assert_eq!(
            &args[..4],
            &["host", "directory", "--path=/host/repo", "--gitignore"]
        );
        assert_eq!(args[4], "--exclude=.git,ui/node_modules");
        assert!(args.contains(&"docker-build".to_string()));
    }

    #[test]
    fn caches_env_and_workdir_come_before_the_commands_which_keep_their_order() {
        let mut spec = spec();
        spec.workdir = "/src/ui".into();
        spec.caches = vec![CacheMount {
            name: "paws-run-x-src-target".into(),
            path: "/src/target".into(),
        }];
        spec.env = vec![("CARGO_BUILD_JOBS".into(), "4".into())];
        spec.commands = vec![
            shell_step("mkdir -p dist, && echo ok"),
            vec!["pnpm".into(), "test".into()],
        ];
        let args = dagger_pipeline_args(&spec).unwrap();
        let position = |needle: &str| args.iter().position(|a| a == needle).unwrap();

        let cache = position("with-mounted-cache");
        let env = position("with-env-variable");
        let workdir = position("--path=/src/ui");
        let first = position("--args=sh,-c,\"mkdir -p dist, && echo ok\"");
        let second = position("--args=pnpm,test");
        assert!(cache < env && env < workdir && workdir < first && first < second);
        assert_eq!(args.last().unwrap(), "stdout");
    }

    #[test]
    fn a_run_with_nothing_to_run_is_an_error() {
        let mut spec = spec();
        spec.commands.clear();
        assert!(dagger_pipeline_args(&spec).is_err());
    }

    #[test]
    fn bad_apt_packages_fail_before_anything_runs() {
        let mut spec = spec();
        spec.apt_packages = vec!["ok-pkg".into(), "bad pkg".into()];
        assert!(dagger_pipeline_args(&spec).is_err());
    }

    #[test]
    fn the_cache_scope_is_the_directory_name() {
        assert_eq!(
            cache_scope(std::path::Path::new("/home/me/fathom")).unwrap(),
            "fathom"
        );
        assert!(cache_scope(std::path::Path::new("/")).is_err());
    }
}
