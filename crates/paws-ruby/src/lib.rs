//! Native Ruby CI support for `paws ci --toolchain ruby`. Like
//! `paws-go`/`paws-java`/`paws-kotlin`, this is not a port: `gh-reusable`'s
//! Dagger module only ever had `setupRuby` (container setup — install a
//! Ruby, nothing else; see `packages/dagger-module/src/index.ts`), with no
//! build/test steps to carry over. The step sequence here
//! (`bundle install`, then the project's own test task) follows ordinary
//! Bundler conventions rather than a `gh-reusable` function.
//!
//! Bundler-only, deliberately: `bundle install` + `bundle exec` is how
//! essentially every real Ruby project (Rails, Sinatra, a plain gem) runs
//! its tests, and a `Gemfile`-less project has no dependency story for
//! this pipeline to honor in the first place. No `gem build` step — a
//! Gemfile alone doesn't imply a gemspec, and `paws publish` is where
//! packaging belongs (see `crates/paws-publish`), not `paws ci`.
//!
//! No `builders/*` image: `ruby:trixie` already ships Ruby, `RubyGems`, and
//! Bundler with a full build toolchain for native gem extensions, so a
//! plain public image pull is enough (the same call `paws-go`/`paws-python`
//! make — see `docs/ROADMAP.md`'s "How a new stack gets added").

use paws_core::Pipeline;
use std::path::Path;

use anyhow::Result;

/// Ruby publishes no LTS tag the way Node does, and no "latest major"
/// alias that lags current the way a pinned `golang:1` does — its unsuffixed
/// Debian-codename tag (`ruby:trixie`) *is* the current stable release,
/// self-updating on each new one. That puts it in the same
/// nothing-to-bump-ever category as `node:lts-trixie`/`golang:1-bookworm`
/// rather than the hardcoded-pin category `astral/uv:python3.13-*` is in
/// (see `docs/ROADMAP.md`'s "Base image version policy"). Trixie, not
/// `ruby:latest`, so the OS base is pinned even while Ruby floats.
pub const BASE_IMAGE: &str = "ruby:trixie";

/// How a project runs its tests. Detected structurally from what the repo
/// actually contains, not from a flag — the two conventions cover
/// effectively every real Bundler project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestRunner {
    /// A `Rakefile` is present: `bundle exec rake` (whose `default` task is
    /// conventionally the test task).
    Rake,
    /// No `Rakefile`, but a `spec/` directory is: `bundle exec rspec`.
    RSpec,
}

impl TestRunner {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Rake => "rake",
            Self::RSpec => "rspec",
        }
    }

    fn args(self) -> Vec<String> {
        match self {
            Self::Rake => vec!["bundle".into(), "exec".into(), "rake".into()],
            Self::RSpec => vec!["bundle".into(), "exec".into(), "rspec".into()],
        }
    }
}

#[derive(Debug)]
pub struct RubyProject {
    /// Whether `Gemfile.lock` is committed. When it is, the install runs
    /// with `BUNDLE_FROZEN=true` so a lockfile that doesn't match the
    /// `Gemfile` fails the build instead of being silently rewritten —
    /// the same "a committed lockfile is a hard constraint in CI" rule
    /// `paws-python`'s `uv sync --frozen` and `paws-node`'s `npm ci`
    /// apply. Set via the env var rather than `bundle install --frozen`
    /// (deprecated in Bundler 2, gone in Bundler 4 — confirmed against
    /// `ruby:trixie`, which ships Bundler 4).
    pub has_lockfile: bool,
    pub test_runner: TestRunner,
}

/// A Bundler project has a `Gemfile` at its root — the file `bundle
/// install`/`bundle exec` both require.
pub fn is_ruby_project(dir: &Path) -> bool {
    dir.join("Gemfile").is_file()
}

pub fn detect_project(dir: &Path) -> Result<RubyProject> {
    if !is_ruby_project(dir) {
        anyhow::bail!("no Gemfile found in {}", dir.display());
    }
    let test_runner = if dir.join("Rakefile").is_file() {
        TestRunner::Rake
    } else if dir.join("spec").is_dir() {
        TestRunner::RSpec
    } else {
        anyhow::bail!(
            "ruby project detected in {}, but neither a Rakefile nor a spec/ directory is present — paws ci --toolchain ruby needs one of them to know how to run the tests",
            dir.display()
        );
    };
    Ok(RubyProject {
        has_lockfile: dir.join("Gemfile.lock").is_file(),
        test_runner,
    })
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`)
/// for `project`: `bundle install` (frozen when a lockfile is committed),
/// then the project's own test task.
pub fn dagger_pipeline_args(project: &RubyProject, source_dir: &str) -> Vec<String> {
    dagger_pipeline_args_with_image(project, source_dir, BASE_IMAGE)
}

/// [`dagger_pipeline_args`] against an explicit image, so a resolved Ruby
/// version (`.ruby-version`, `paws.toml`, `--toolchain-version`) reaches the
/// build — see `paws_core::Toolchain::image_for`.
pub fn dagger_pipeline_args_with_image(
    project: &RubyProject,
    source_dir: &str,
    image: &str,
) -> Vec<String> {
    Pipeline::from_image(image)
        .mount("/src", source_dir)
        .workdir("/src")
        .env_if(project.has_lockfile, "BUNDLE_FROZEN", "true")
        .exec(["bundle", "install"])
        .exec(project.test_runner.args())
        .stdout()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        paws_core::test_support::scratch_dir("ruby", name)
    }

    #[test]
    fn detects_ruby_project_from_gemfile() {
        let dir = temp_dir("detect");
        assert!(!is_ruby_project(&dir));
        fs::write(dir.join("Gemfile"), "source \"https://rubygems.org\"\n").unwrap();
        assert!(is_ruby_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_when_no_gemfile() {
        let dir = temp_dir("no-gemfile");
        assert!(detect_project(&dir).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_when_no_test_task_is_discoverable() {
        let dir = temp_dir("no-tests");
        fs::write(dir.join("Gemfile"), "").unwrap();
        let err = detect_project(&dir).unwrap_err().to_string();
        assert!(
            err.contains("Rakefile") && err.contains("spec/"),
            "error should name both conventions it looked for: {err}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn prefers_rake_over_rspec_when_both_are_present() {
        let dir = temp_dir("rake-wins");
        fs::write(dir.join("Gemfile"), "").unwrap();
        fs::write(dir.join("Rakefile"), "").unwrap();
        fs::create_dir_all(dir.join("spec")).unwrap();
        assert_eq!(detect_project(&dir).unwrap().test_runner, TestRunner::Rake);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn falls_back_to_rspec_without_a_rakefile() {
        let dir = temp_dir("rspec");
        fs::write(dir.join("Gemfile"), "").unwrap();
        fs::create_dir_all(dir.join("spec")).unwrap();
        let project = detect_project(&dir).unwrap();
        assert_eq!(project.test_runner, TestRunner::RSpec);
        let args = dagger_pipeline_args(&project, "/host/src");
        assert!(args.contains(&"--args=bundle,exec,rspec".to_string()));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_lockfile_presence() {
        let dir = temp_dir("lockfile");
        fs::write(dir.join("Gemfile"), "").unwrap();
        fs::write(dir.join("Rakefile"), "").unwrap();
        assert!(!detect_project(&dir).unwrap().has_lockfile);
        fs::write(dir.join("Gemfile.lock"), "").unwrap();
        assert!(detect_project(&dir).unwrap().has_lockfile);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pipeline_uses_the_ruby_image_and_runs_install_then_tests() {
        let project = RubyProject {
            has_lockfile: false,
            test_runner: TestRunner::Rake,
        };
        let args = dagger_pipeline_args(&project, "/host/src");
        assert_eq!(args[0], "container");
        assert_eq!(args[1], "from");
        assert_eq!(args[2], "--address=ruby:trixie");
        assert!(args.contains(&"--args=bundle,install".to_string()));
        assert!(args.contains(&"--args=bundle,exec,rake".to_string()));
        assert_eq!(args.last(), Some(&"stdout".to_string()));
    }

    #[test]
    fn pipeline_sets_bundle_frozen_only_with_a_lockfile() {
        let without = dagger_pipeline_args(
            &RubyProject {
                has_lockfile: false,
                test_runner: TestRunner::Rake,
            },
            "/host/src",
        );
        assert!(!without.iter().any(|a| a.contains("BUNDLE_FROZEN")));

        let with = dagger_pipeline_args(
            &RubyProject {
                has_lockfile: true,
                test_runner: TestRunner::Rake,
            },
            "/host/src",
        );
        assert!(with.contains(&"--name=BUNDLE_FROZEN".to_string()));
        assert!(with.contains(&"--value=true".to_string()));
    }
}
