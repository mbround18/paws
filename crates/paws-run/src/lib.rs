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

use anyhow::{Result, bail};
pub use paws_core::container::{
    CacheMount, DEFAULT_EXCLUDES, Export, SOURCE_ROOT, cache_scope, container_path,
    container_workdir, parse_cache, parse_env, parse_export, validate_apt_package, validate_setup,
};
use paws_core::{Base, ContainerOptions};

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
    /// A directory copied back to the host after the last command.
    pub export: Option<Export>,
}

impl RunSpec {
    fn container_options(&self) -> ContainerOptions {
        ContainerOptions {
            apt_packages: self.apt_packages.clone(),
            setup: self.setup.clone(),
            env: self.env.clone(),
            caches: self.caches.clone(),
            excludes: self.excludes.clone(),
            export: self.export.clone(),
        }
    }
}

/// The Dockerfile a run is built from: the image, then the Debian packages
/// and the `--setup` commands (before the source copy, so changing a source
/// file never reruns them), then the source.
pub fn dockerfile(image: &str, apt_packages: &[String], setup: &[String]) -> String {
    Base::Image(image).dockerfile(apt_packages, setup)
}

/// Builds the `dagger core <chain>` argument list for `spec`: the filtered
/// source built into the image, then caches, environment and working
/// directory, then each command as its own `with-exec`, ending in the last
/// command's stdout or, with an export, the exported directory.
pub fn dagger_pipeline_args(spec: &RunSpec) -> Result<Vec<String>> {
    if spec.commands.is_empty() {
        bail!("nothing to run: pass --step \"<shell>\" and/or a command after --");
    }
    let options = spec.container_options();
    let mut pipeline = options
        .open(&spec.context_dir, &Base::Image(&spec.image))?
        .workdir(&spec.workdir);
    for command in &spec.commands {
        pipeline = pipeline.exec(command);
    }
    Ok(options.finish(pipeline))
}

/// A `--step` script as a command: `sh -c <script>`.
pub fn shell_step(script: &str) -> Vec<String> {
    vec!["sh".into(), "-c".into(), script.into()]
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
            export: None,
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
    fn an_export_replaces_stdout_with_a_directory_export_after_the_commands() {
        let mut spec = spec();
        spec.export = Some(Export {
            path: "/src/target/release/bundle".into(),
            destination: "/host/out".into(),
        });
        let args = dagger_pipeline_args(&spec).unwrap();
        let position = |needle: &str| args.iter().position(|a| a == needle).unwrap();
        assert!(position("--args=cargo,test") < position("export"));
        assert_eq!(
            &args[args.len() - 4..],
            &[
                "directory",
                "--path=/src/target/release/bundle",
                "export",
                "--path=/host/out"
            ]
        );
        assert!(!args.iter().any(|a| a == "stdout"));
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
}
