//! The container shape shared by `paws run` and every `paws ci` toolchain
//! that builds from a filtered copy of the source: a base (an image, or a
//! builder Dockerfile), Debian packages and setup commands baked in before
//! the source, the source itself filtered on the host by `.gitignore`, then
//! cache volumes, environment variables and, at the end, either the last
//! step's stdout or a directory exported back to the host.
//!
//! `paws run` had this shape first; moving it here lets `paws ci --toolchain
//! rust|tauri` offer the same knobs (`--apt`, `--setup`, `--env`, `--cache`,
//! `--exclude`, `--export`) instead of each toolchain growing its own.

use anyhow::{Context, Result, bail};
use std::path::Path;

use crate::Pipeline;

/// Where the source directory lands inside the container.
pub const SOURCE_ROOT: &str = "/src";

/// Excluded from every context on top of `.gitignore` and `--exclude`.
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

/// A directory copied back out of the container once the pipeline ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Export {
    /// Absolute container path.
    pub path: String,
    /// Absolute host path; created or replaced.
    pub destination: String,
}

/// Everything a caller can add to a toolchain's own recipe.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContainerOptions {
    /// Debian packages installed in their own image layer, before the source.
    pub apt_packages: Vec<String>,
    /// Shell commands baked into the image after the packages, each its own
    /// `RUN` layer.
    pub setup: Vec<String>,
    pub env: Vec<(String, String)>,
    pub caches: Vec<CacheMount>,
    /// Extra patterns left out of the context, beyond `.gitignore`.
    pub excludes: Vec<String>,
    /// Replaces the final `stdout` with a directory export.
    pub export: Option<Export>,
}

impl ContainerOptions {
    /// Fails on anything that would be spliced into the Dockerfile as more
    /// than one package name or one `RUN` line.
    pub fn validate(&self) -> Result<()> {
        for package in &self.apt_packages {
            validate_apt_package(package)?;
        }
        for script in &self.setup {
            validate_setup(script)?;
        }
        Ok(())
    }

    /// [`DEFAULT_EXCLUDES`] followed by the caller's own.
    pub fn context_excludes(&self) -> Vec<String> {
        let mut excludes: Vec<String> = DEFAULT_EXCLUDES.iter().map(ToString::to_string).collect();
        excludes.extend(self.excludes.iter().cloned());
        excludes
    }

    /// Opens a pipeline on the filtered `context_dir`, built from `base` (an
    /// image reference, or a whole builder Dockerfile) plus these options'
    /// packages and setup, with the caches and environment applied. The
    /// caller adds its steps and ends with [`ContainerOptions::finish`].
    pub fn open(&self, context_dir: &str, base: &Base<'_>) -> Result<Pipeline> {
        self.validate()?;
        let dockerfile = base.dockerfile(&self.apt_packages, &self.setup);
        let mut pipeline = Pipeline::from_host_context_with_build_args(
            context_dir,
            &self.context_excludes(),
            &dockerfile,
            base.build_args(),
        );
        for cache in &self.caches {
            pipeline = pipeline.mount_cache(&cache.path, &cache.name);
        }
        for (name, value) in &self.env {
            pipeline = pipeline.env(name, value);
        }
        Ok(pipeline)
    }

    /// Terminates `pipeline`: an export when one was asked for, otherwise the
    /// last step's stdout.
    pub fn finish(&self, pipeline: Pipeline) -> Vec<String> {
        match &self.export {
            Some(export) if self.in_cache(&export.path) => pipeline
                .exec([
                    "sh",
                    "-c",
                    &format!(
                        "rm -rf {EXPORT_STAGING} && cp -a {} {EXPORT_STAGING}",
                        export.path
                    ),
                ])
                .export_directory(EXPORT_STAGING, &export.destination),
            Some(export) => pipeline.export_directory(&export.path, &export.destination),
            None => pipeline.stdout(),
        }
    }

    /// Whether `path` lives under one of the cache volumes. Dagger cannot
    /// read a directory out of a cache mount (`cannot retrieve path from
    /// cache`), so an export from there — a Tauri bundle under a cached
    /// `target/` is the common case — is copied to [`EXPORT_STAGING`] first.
    fn in_cache(&self, path: &str) -> bool {
        self.caches.iter().any(|cache| {
            let root = cache.path.trim_end_matches('/');
            path == root || path.starts_with(&format!("{root}/"))
        })
    }
}

/// Where an export from inside a cache volume is copied before Dagger reads
/// it; see [`ContainerOptions::finish`].
pub const EXPORT_STAGING: &str = "/.paws-export";

/// What the container is built from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base<'a> {
    /// A pulled image: `FROM <image>`.
    Image(&'a str),
    /// A complete builder Dockerfile (one of `builders/*`, embedded), built
    /// with the standard provenance build args.
    Dockerfile(&'a str),
}

impl Base<'_> {
    /// The Dockerfile for this base plus `apt_packages` and `setup`, each
    /// before the source copy so changing a source file never reruns them.
    pub fn dockerfile(&self, apt_packages: &[String], setup: &[String]) -> String {
        let mut lines = match self {
            Base::Image(image) => vec![format!("FROM {image}")],
            Base::Dockerfile(text) => vec![text.trim_end().to_string()],
        };
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

    fn build_args(&self) -> Option<String> {
        match self {
            Base::Image(_) => None,
            Base::Dockerfile(_) => Some(crate::builder_build_args()),
        }
    }
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

/// An absolute container path for `path`: absolute ones are kept, relative
/// ones are resolved against `workdir`, and `.`/`..` segments are folded so
/// `../target/release/bundle` from `/src/ui` is `/src/target/release/bundle`.
pub fn container_path(workdir: &str, path: &str) -> Result<String> {
    let joined = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("{}/{path}", workdir.trim_end_matches('/'))
    };
    let mut parts: Vec<&str> = Vec::new();
    for part in joined.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    bail!("{path:?} climbs above the container's root");
                }
            }
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        bail!("{path:?} is the container's root, which cannot be exported");
    }
    Ok(format!("/{}", parts.join("/")))
}

/// Parses one `--export` entry, `CONTAINER_PATH=HOST_PATH`. The container
/// path follows [`container_path`] against `workdir`; a relative host path is
/// relative to `host_base` (the caller's working directory).
pub fn parse_export(entry: &str, workdir: &str, host_base: &Path) -> Result<Export> {
    let Some((path, destination)) = entry.split_once('=') else {
        bail!("--export {entry:?} must be CONTAINER_PATH=HOST_PATH");
    };
    if path.is_empty() || destination.is_empty() {
        bail!("--export {entry:?} must be CONTAINER_PATH=HOST_PATH");
    }
    let destination = Path::new(destination);
    let destination = if destination.is_absolute() {
        destination.to_path_buf()
    } else {
        host_base.join(destination)
    };
    Ok(Export {
        path: container_path(workdir, path)?,
        destination: destination.to_string_lossy().into_owned(),
    })
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

/// The name `--cache` volumes are scoped by: the source directory's own name.
pub fn cache_scope(context_dir: &Path) -> Result<String> {
    context_dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .context("the source directory has no name to scope --cache volumes by")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_base_installs_packages_before_copying_the_source() {
        let file = Base::Image("rust:1-bookworm").dockerfile(
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
    fn a_dockerfile_base_is_kept_whole_and_extended_at_the_end() {
        let file =
            Base::Dockerfile("# syntax=docker/dockerfile:1.7\nFROM rust:1-bookworm\nRUN true\n")
                .dockerfile(&["cmake".into()], &[]);
        let lines: Vec<&str> = file.lines().collect();
        assert_eq!(lines[0], "# syntax=docker/dockerfile:1.7");
        assert_eq!(lines[2], "RUN true");
        assert!(lines[3].contains("cmake"));
        assert_eq!(lines[4], "WORKDIR /src");
        assert_eq!(lines[5], "COPY . /src");
    }

    #[test]
    fn no_packages_means_no_apt_layer() {
        assert!(
            !Base::Image("alpine:3")
                .dockerfile(&[], &[])
                .contains("apt-get")
        );
    }

    #[test]
    fn open_applies_excludes_caches_and_env_and_finish_picks_the_terminator() {
        let options = ContainerOptions {
            excludes: vec!["ui/node_modules".into()],
            caches: vec![CacheMount {
                name: "c".into(),
                path: "/src/target".into(),
            }],
            env: vec![("CI".into(), "true".into())],
            ..Default::default()
        };
        let pipeline = options
            .open("/host/repo", &Base::Image("rust:1-bookworm"))
            .unwrap();
        let args = options.finish(pipeline.clone());
        assert_eq!(
            &args[..5],
            &[
                "host",
                "directory",
                "--path=/host/repo",
                "--gitignore",
                "--exclude=.git,ui/node_modules"
            ]
        );
        assert!(!args.iter().any(|a| a.starts_with("--build-args=")));
        let position = |needle: &str| args.iter().position(|a| a == needle).unwrap();
        assert!(position("with-mounted-cache") < position("with-env-variable"));
        assert_eq!(args.last().unwrap(), "stdout");

        let exporting = ContainerOptions {
            export: Some(Export {
                path: "/src/target/release/bundle".into(),
                destination: "/host/out".into(),
            }),
            ..options
        };
        let args = exporting.finish(pipeline.clone());
        assert_eq!(
            &args[args.len() - 6..],
            &[
                "with-exec",
                "--args=sh,-c,rm -rf /.paws-export && cp -a /src/target/release/bundle /.paws-export",
                "directory",
                "--path=/.paws-export",
                "export",
                "--path=/host/out"
            ][..],
            "an export from inside a cache volume is staged first"
        );

        let exporting = ContainerOptions {
            export: Some(Export {
                path: "/src/dist".into(),
                destination: "/host/out".into(),
            }),
            ..exporting
        };
        let args = exporting.finish(pipeline);
        assert_eq!(
            &args[args.len() - 4..],
            &[
                "directory",
                "--path=/src/dist",
                "export",
                "--path=/host/out"
            ]
        );
    }

    #[test]
    fn a_dockerfile_base_builds_with_provenance_args() {
        let args = ContainerOptions::default()
            .open("/host/repo", &Base::Dockerfile("FROM rust:1-bookworm\n"))
            .unwrap()
            .stdout();
        assert!(
            args.iter()
                .any(|a| a.starts_with("--build-args=BUILDER_VERSION="))
        );
    }

    #[test]
    fn open_refuses_bad_packages_and_multi_line_setup() {
        let bad_apt = ContainerOptions {
            apt_packages: vec!["bad pkg".into()],
            ..Default::default()
        };
        assert!(bad_apt.open("/x", &Base::Image("a")).is_err());
        let bad_setup = ContainerOptions {
            setup: vec!["echo a\nFROM evil".into()],
            ..Default::default()
        };
        assert!(bad_setup.open("/x", &Base::Image("a")).is_err());
        validate_setup("apt-get update && apt-get install -y x").unwrap();
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
    fn container_paths_resolve_against_the_workdir_and_fold_dot_segments() {
        assert_eq!(
            container_path("/src/ui", "../target/release/bundle").unwrap(),
            "/src/target/release/bundle"
        );
        assert_eq!(container_path("/src", "dist/").unwrap(), "/src/dist");
        assert_eq!(container_path("/src", "/out/./x").unwrap(), "/out/x");
        assert!(container_path("/src", "../../x").is_err());
        assert!(container_path("/src", "/").is_err());
    }

    #[test]
    fn exports_pair_a_container_path_with_an_absolute_host_path() {
        let export = parse_export(
            "../target/release/bundle=dist/bundles",
            "/src/ui",
            Path::new("/home/me/repo"),
        )
        .unwrap();
        assert_eq!(export.path, "/src/target/release/bundle");
        assert_eq!(export.destination, "/home/me/repo/dist/bundles");
        let absolute = parse_export("/src/dist=/tmp/out", "/src", Path::new("/x")).unwrap();
        assert_eq!(absolute.destination, "/tmp/out");
        assert!(parse_export("no-equals", "/src", Path::new("/x")).is_err());
        assert!(parse_export("=/out", "/src", Path::new("/x")).is_err());
        assert!(parse_export("dist=", "/src", Path::new("/x")).is_err());
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
    fn the_cache_scope_is_the_directory_name() {
        assert_eq!(cache_scope(Path::new("/home/me/fathom")).unwrap(), "fathom");
        assert!(cache_scope(Path::new("/")).is_err());
    }
}
