//! Builds `dagger core` argument vectors.
//!
//! Every language crate used to assemble the same chain by hand — `container`
//! → `from` → `with-mounted-directory` → `with-workdir` → `with-exec`… →
//! `stdout` — as a `Vec<String>` of positional flags, sixteen times over. The
//! sequence is identical in all of them; only the base image and the commands
//! differ, which is exactly what a builder takes as arguments.
//!
//! This lives in `paws-core` rather than `paws-dagger` so a crate that only
//! needs to *describe* a pipeline doesn't take on the HTTP client and async
//! runtime needed to *run* one. `paws-dagger` re-exports it for call sites
//! that already depend on the executor.
//!
//! ## The `--args` comma convention
//!
//! `dagger core with-exec` takes its command as one comma-joined
//! `--args=a,b,c` value, so an argument containing a literal comma would be
//! split into two. That constraint is inherited from `dagger core`, not
//! introduced here; centralizing it means there is now one place to fix it if
//! `dagger` ever grows a repeatable flag, and one place that checks for it —
//! see [`Pipeline::exec`].

/// The provenance `--build-args` every `builders/*` image is built with.
///
/// `dev`/`unknown` for version and revision, and the current Unix time for
/// the created stamp — the same five-line block that was written out in
/// `paws-java`, `paws-kotlin`, `paws-tauri`, `paws-flatpak` and `paws-esp32`
/// independently, which meant a change to the label convention had to be
/// made five times.
pub fn builder_build_args() -> String {
    let created_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    format!("BUILDER_VERSION=dev,BUILDER_REVISION=unknown,BUILDER_CREATED={created_unix}")
}

/// A `dagger core` invocation under construction.
///
/// Consuming builder: each step returns `self`, and [`Pipeline::stdout`] (or
/// another terminal call) yields the argument vector to hand to
/// `paws_dagger::core`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pipeline {
    args: Vec<String>,
}

impl Pipeline {
    /// `container from --address=<image>` — a pipeline against a pulled image.
    pub fn from_image(image: &str) -> Self {
        Self {
            args: vec![
                "container".into(),
                "from".into(),
                format!("--address={image}"),
            ],
        }
    }

    /// `host directory --path=<dir> docker-build` — a pipeline against an
    /// image built from a Dockerfile directory on the host, which is how
    /// every toolchain that materializes one of `builders/*` starts.
    pub fn from_host_dockerfile(builder_dir: &str) -> Self {
        Self {
            args: vec![
                "host".into(),
                "directory".into(),
                format!("--path={builder_dir}"),
                "docker-build".into(),
            ],
        }
    }

    /// [`Pipeline::from_host_dockerfile`] with `--build-args=<args>`, for the
    /// builders that bake provenance labels into the image.
    ///
    /// `build_args` is passed through verbatim: it is `dagger core`'s own
    /// comma-separated `KEY=value` list, not a command, so the comma check in
    /// [`Pipeline::exec`] does not apply to it.
    pub fn from_host_dockerfile_with_build_args(builder_dir: &str, build_args: &str) -> Self {
        let mut pipeline = Self::from_host_dockerfile(builder_dir);
        pipeline.args.push(format!("--build-args={build_args}"));
        pipeline
    }

    /// [`Pipeline::from_host_dockerfile`] with the standard provenance
    /// [`builder_build_args`] — how every `builders/*` toolchain opens.
    pub fn from_builder_image(builder_dir: &str) -> Self {
        Self::from_host_dockerfile_with_build_args(builder_dir, &builder_build_args())
    }

    /// Starts from a caller-supplied prefix, for the few chains that open
    /// with something this builder doesn't model yet. Prefer the constructors
    /// above; this exists so adopting the builder never requires modelling
    /// every exotic opening at once.
    pub fn from_raw(args: impl IntoIterator<Item = String>) -> Self {
        Self {
            args: args.into_iter().collect(),
        }
    }

    /// `with-mounted-directory --path=<path> --source=<source>`
    pub fn mount(mut self, path: &str, source: &str) -> Self {
        self.args.push("with-mounted-directory".into());
        self.args.push(format!("--path={path}"));
        self.args.push(format!("--source={source}"));
        self
    }

    /// `with-workdir --path=<path>`
    pub fn workdir(mut self, path: &str) -> Self {
        self.args.push("with-workdir".into());
        self.args.push(format!("--path={path}"));
        self
    }

    /// `with-env-variable --name=<name> --value=<value>`
    pub fn env(mut self, name: &str, value: &str) -> Self {
        self.args.push("with-env-variable".into());
        self.args.push(format!("--name={name}"));
        self.args.push(format!("--value={value}"));
        self
    }

    /// `with-env-variable` only when `condition` holds — the shape of every
    /// "set this only for a lockfile/wasm/frozen build" branch.
    pub fn env_if(self, condition: bool, name: &str, value: &str) -> Self {
        if condition {
            self.env(name, value)
        } else {
            self
        }
    }

    /// `with-exec --args=<command joined by commas>`
    ///
    /// Debug builds assert that no single argument contains a comma, since
    /// `dagger core` would silently split it into two arguments and the
    /// failure would surface as a confusing error from the tool being run
    /// rather than from here.
    pub fn exec<I, S>(mut self, command: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let parts: Vec<String> = command
            .into_iter()
            .map(|part| {
                let part = part.as_ref();
                debug_assert!(
                    !part.contains(','),
                    "dagger core joins --args with commas, so {part:?} would be split in two"
                );
                part.to_string()
            })
            .collect();
        self.args.push("with-exec".into());
        self.args.push(format!("--args={}", parts.join(",")));
        self
    }

    /// `with-exec` only when `condition` holds — the shape of every "run lint
    /// only if the project has one" branch.
    pub fn exec_if<I, S>(self, condition: bool, command: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        if condition { self.exec(command) } else { self }
    }

    /// `with-exec --insecure-root-capabilities --args=…` — a step that needs
    /// real root inside the container.
    ///
    /// Only `flatpak` uses this: `flatpak-builder`'s own sandboxed build is
    /// FUSE-backed and a bare device flag isn't enough. It is deliberately a
    /// separate, explicitly-named method rather than a flag on
    /// [`Pipeline::exec`], so granting root is always visible at the call site.
    pub fn exec_as_root<I, S>(mut self, command: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self = self.exec(command);
        // `--insecure-root-capabilities` sits between `with-exec` and its
        // `--args`, so splice it in rather than appending.
        let args_index = self.args.len() - 1;
        self.args
            .insert(args_index, "--insecure-root-capabilities".into());
        self
    }

    /// `with-secret-variable --name=<name> --secret=env:<name>` — passes a
    /// credential in from the host environment without it appearing in the
    /// build log or the image layers.
    pub fn secret_env(mut self, name: &str) -> Self {
        self.args.push("with-secret-variable".into());
        self.args.push(format!("--name={name}"));
        self.args.push(format!("--secret=env:{name}"));
        self
    }

    /// `with-new-file --path=<path> --contents=<contents>` — writes a file
    /// into the container without it having to exist on the host.
    pub fn new_file(mut self, path: &str, contents: &str) -> Self {
        self.args.push("with-new-file".into());
        self.args.push(format!("--path={path}"));
        self.args.push(format!("--contents={contents}"));
        self
    }

    /// `with-exec --expect=ANY --args=…` — a step whose non-zero exit is a
    /// result to read, not a pipeline failure.
    ///
    /// A security scanner exits non-zero precisely when it has findings, so
    /// the report still has to be collected afterwards.
    pub fn exec_expecting_any_exit<I, S>(mut self, command: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self = self.exec(command);
        let args_index = self.args.len() - 1;
        self.args.insert(args_index, "--expect=ANY".into());
        self
    }

    /// Terminates with `file --path=<path> contents`, yielding a file written
    /// inside the container rather than the last step's stdout.
    pub fn file_contents(mut self, path: &str) -> Vec<String> {
        self.args.push("file".into());
        self.args.push(format!("--path={path}"));
        self.args.push("contents".into());
        self.args
    }

    /// Appends raw arguments, for a step this builder doesn't model.
    pub fn raw(mut self, args: impl IntoIterator<Item = String>) -> Self {
        self.args.extend(args);
        self
    }

    /// Terminates the chain with `stdout` and yields the argument vector.
    pub fn stdout(mut self) -> Vec<String> {
        self.args.push("stdout".into());
        self.args
    }

    /// Terminates with `directory --path=<path> export --path=<destination>`,
    /// the chain that copies a built directory back out to the host.
    pub fn export_directory(mut self, path: &str, destination: &str) -> Vec<String> {
        self.args.push("directory".into());
        self.args.push(format!("--path={path}"));
        self.args.push("export".into());
        self.args.push(format!("--path={destination}"));
        self.args
    }

    /// Yields the arguments with no terminal call appended.
    pub fn into_args(self) -> Vec<String> {
        self.args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_build_and_test_chain_matches_the_hand_written_form() {
        let args = Pipeline::from_image("golang:1-bookworm")
            .mount("/src", "/host/src")
            .workdir("/src")
            .exec(["go", "build", "./..."])
            .exec(["go", "test", "./..."])
            .stdout();

        assert_eq!(
            args,
            vec![
                "container",
                "from",
                "--address=golang:1-bookworm",
                "with-mounted-directory",
                "--path=/src",
                "--source=/host/src",
                "with-workdir",
                "--path=/src",
                "with-exec",
                "--args=go,build,./...",
                "with-exec",
                "--args=go,test,./...",
                "stdout",
            ]
        );
    }

    #[test]
    fn conditional_steps_are_omitted_entirely_when_false() {
        let without = Pipeline::from_image("ruby:3-bookworm")
            .env_if(false, "BUNDLE_FROZEN", "true")
            .exec_if(false, ["bundle", "exec", "rubocop"])
            .stdout();
        assert!(!without.iter().any(|a| a.contains("BUNDLE_FROZEN")));
        assert!(!without.iter().any(|a| a.contains("rubocop")));

        let with = Pipeline::from_image("ruby:3-bookworm")
            .env_if(true, "BUNDLE_FROZEN", "true")
            .stdout();
        assert_eq!(
            with,
            vec![
                "container",
                "from",
                "--address=ruby:3-bookworm",
                "with-env-variable",
                "--name=BUNDLE_FROZEN",
                "--value=true",
                "stdout",
            ]
        );
    }

    #[test]
    fn a_builder_dir_chain_opens_by_docker_building_the_host_directory() {
        let args = Pipeline::from_host_dockerfile("/tmp/paws-builders/rust")
            .mount("/src", "/host/src")
            .workdir("/src")
            .stdout();
        assert_eq!(
            &args[..4],
            &[
                "host",
                "directory",
                "--path=/tmp/paws-builders/rust",
                "docker-build",
            ]
        );
    }

    #[test]
    fn build_args_follow_docker_build_and_are_not_comma_checked() {
        let args = Pipeline::from_host_dockerfile_with_build_args(
            "/tmp/paws-builders/java",
            "BUILDER_VERSION=dev,BUILDER_REVISION=unknown",
        )
        .stdout();
        assert_eq!(
            &args[..5],
            &[
                "host",
                "directory",
                "--path=/tmp/paws-builders/java",
                "docker-build",
                "--build-args=BUILDER_VERSION=dev,BUILDER_REVISION=unknown",
            ]
        );
    }

    #[test]
    fn export_directory_terminates_with_the_export_chain() {
        let args = Pipeline::from_image("rust:1-bookworm")
            .workdir("/src")
            .export_directory("/src/target/release", "/host/out");
        assert_eq!(
            &args[args.len() - 4..],
            &[
                "directory",
                "--path=/src/target/release",
                "export",
                "--path=/host/out",
            ]
        );
    }

    #[test]
    fn exec_accepts_owned_and_borrowed_command_parts() {
        let owned: Vec<String> = vec!["mix".into(), "test".into()];
        let from_owned = Pipeline::from_image("elixir:1").exec(owned).into_args();
        let from_borrowed = Pipeline::from_image("elixir:1")
            .exec(["mix", "test"])
            .into_args();
        assert_eq!(from_owned, from_borrowed);
    }

    #[test]
    fn exec_as_root_puts_the_flag_before_the_args() {
        let args = Pipeline::from_image("ubuntu:26.04")
            .exec_as_root(["flatpak-builder", "--build-only"])
            .stdout();
        assert_eq!(
            args,
            vec![
                "container",
                "from",
                "--address=ubuntu:26.04",
                "with-exec",
                "--insecure-root-capabilities",
                "--args=flatpak-builder,--build-only",
                "stdout",
            ]
        );
    }

    #[test]
    fn secret_env_never_puts_the_value_in_the_argument_vector() {
        let args = Pipeline::from_image("rust:1-bookworm")
            .secret_env("CARGO_REGISTRY_TOKEN")
            .stdout();
        assert!(args.contains(&"--secret=env:CARGO_REGISTRY_TOKEN".to_string()));
        assert!(
            args.iter().all(|a| !a.contains("secret-value")),
            "only the variable name travels in the args"
        );
    }

    #[test]
    fn builder_images_carry_the_standard_provenance_build_args() {
        let args = Pipeline::from_builder_image("/tmp/paws-builders/esp32").into_args();
        let build_args = args
            .iter()
            .find(|a| a.starts_with("--build-args="))
            .expect("a builder image is built with provenance args");
        assert!(build_args.contains("BUILDER_VERSION=dev"));
        assert!(build_args.contains("BUILDER_REVISION=unknown"));
        assert!(build_args.contains("BUILDER_CREATED="));
    }

    #[test]
    fn a_scanner_chain_writes_its_script_then_tolerates_a_non_zero_exit() {
        let args = Pipeline::from_image("returntocorp/semgrep:1.81.0")
            .mount("/src", "/host/src")
            .workdir("/src")
            .new_file("/scan.sh", "semgrep scan")
            .exec_expecting_any_exit(["sh", "/scan.sh"])
            .file_contents("/report.json");
        assert_eq!(
            &args[args.len() - 9..],
            &[
                "with-new-file",
                "--path=/scan.sh",
                "--contents=semgrep scan",
                "with-exec",
                "--expect=ANY",
                "--args=sh,/scan.sh",
                "file",
                "--path=/report.json",
                "contents",
            ]
        );
    }

    #[test]
    #[should_panic(expected = "would be split in two")]
    fn a_comma_in_a_command_argument_is_caught_in_debug_builds() {
        Pipeline::from_image("rust:1-bookworm").exec(["cargo", "build", "--features=a,b"]);
    }
}
