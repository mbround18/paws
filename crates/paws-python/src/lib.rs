//! Native Python CI support, ported from `gh-reusable`'s `pythonBuildAndTest`
//! Dagger function for parity (`packages/dagger-module/src/index.ts`):
//! `uv sync --all-groups [--frozen]`, `uv build`, `uv run pytest`, against
//! the `astral/uv:python<version>-trixie-slim` image. Only `uv`-based
//! projects (a `pyproject.toml`) are supported — `gh-reusable` never had a
//! poetry/pipenv/pip path to port, and inventing one here would be exactly
//! the "reimplementation from memory" the project's parity principle exists
//! to avoid.

use paws_core::Pipeline;
use std::path::Path;

use anyhow::Result;

/// `CPython` has no LTS branding the way Node/Java do — just a rolling
/// "current stable" minor version, each supported for ~5 years. `3.13` is
/// that current stable as of this pin; unlike `node:lts-trixie`, there's no
/// self-updating "latest" tag `astral/uv` publishes for this
/// (`python3-trixie-slim` doesn't exist, confirmed directly against Docker
/// Hub — every tag pins an exact minor), so this needs a real version
/// bump when Python ships a new one, same as any other pinned dependency.
pub const DEFAULT_PYTHON_VERSION: &str = "3.13";

fn base_image(python_version: &str) -> String {
    format!("astral/uv:python{python_version}-trixie-slim")
}

#[derive(Debug, Clone)]
pub struct PythonProject {
    /// Whether `uv.lock` is committed. `uv sync --frozen` (matching
    /// `gh-reusable`'s pipeline) requires it and errors otherwise — verified
    /// directly (`uv sync --all-groups --frozen` with no `uv.lock` present
    /// fails with "Unable to find lockfile"), the same real failure mode
    /// `npm ci`/etc. have without their lockfile (see `paws-node`). A repo
    /// without one yet gets the plain `uv sync --all-groups` instead.
    pub has_lockfile: bool,
}

/// A `uv`-based Python project has a `pyproject.toml` at its root — the
/// file `uv sync`/`uv build` both require.
pub fn is_python_project(dir: &Path) -> bool {
    dir.join("pyproject.toml").is_file()
}

pub fn detect_project(dir: &Path) -> Result<PythonProject> {
    if !is_python_project(dir) {
        anyhow::bail!("no pyproject.toml found in {}", dir.display());
    }
    Ok(PythonProject {
        has_lockfile: dir.join("uv.lock").is_file(),
    })
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`)
/// for `project`, using [`DEFAULT_PYTHON_VERSION`] — matching
/// `gh-reusable`'s own default parameter value.
pub fn dagger_pipeline_args(project: &PythonProject, source_dir: &str) -> Vec<String> {
    dagger_pipeline_args_with_version(project, source_dir, DEFAULT_PYTHON_VERSION)
}

/// Same as [`dagger_pipeline_args`], with an explicit Python version
/// (selects the `astral/uv:python<version>-trixie-slim` image tag).
pub fn dagger_pipeline_args_with_version(
    project: &PythonProject,
    source_dir: &str,
    python_version: &str,
) -> Vec<String> {
    dagger_pipeline_args_with_image(project, source_dir, &base_image(python_version))
}

/// [`dagger_pipeline_args`] against an explicit image — see
/// `paws_core::Toolchain::image_for`.
pub fn dagger_pipeline_args_with_image(
    project: &PythonProject,
    source_dir: &str,
    image: &str,
) -> Vec<String> {
    let mut sync = vec!["uv", "sync", "--all-groups"];
    if project.has_lockfile {
        sync.push("--frozen");
    }

    Pipeline::from_image(image)
        .mount("/src", source_dir)
        .workdir("/src")
        .exec(sync)
        .exec(["uv", "build"])
        .exec(["uv", "run", "pytest"])
        .stdout()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        paws_core::test_support::scratch_dir("python", name)
    }

    #[test]
    fn detects_python_project_from_pyproject_toml() {
        let dir = temp_dir("detect");
        assert!(
            !is_python_project(&dir),
            "should not detect before pyproject.toml exists"
        );
        fs::write(dir.join("pyproject.toml"), "[project]\nname = \"x\"\n").unwrap();
        assert!(is_python_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_when_no_pyproject_toml() {
        let dir = temp_dir("no-pyproject");
        assert!(detect_project(&dir).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_lockfile_presence() {
        let dir = temp_dir("lockfile");
        fs::write(dir.join("pyproject.toml"), "[project]\nname = \"x\"\n").unwrap();
        assert!(!detect_project(&dir).unwrap().has_lockfile);
        fs::write(dir.join("uv.lock"), "").unwrap();
        assert!(detect_project(&dir).unwrap().has_lockfile);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pipeline_uses_the_default_python_image() {
        let project = PythonProject { has_lockfile: true };
        let args = dagger_pipeline_args(&project, "/host/src");
        assert_eq!(args[0], "container");
        assert_eq!(args[1], "from");
        assert_eq!(args[2], "--address=astral/uv:python3.13-trixie-slim");
    }

    #[test]
    fn pipeline_runs_frozen_sync_when_a_lockfile_exists() {
        let project = PythonProject { has_lockfile: true };
        let args = dagger_pipeline_args(&project, "/host/src");
        assert!(args.contains(&"--args=uv,sync,--all-groups,--frozen".to_string()));
        assert!(args.contains(&"--args=uv,build".to_string()));
        assert!(args.contains(&"--args=uv,run,pytest".to_string()));
        assert_eq!(args.last(), Some(&"stdout".to_string()));
    }

    #[test]
    fn pipeline_omits_frozen_flag_without_a_lockfile() {
        let project = PythonProject {
            has_lockfile: false,
        };
        let args = dagger_pipeline_args(&project, "/host/src");
        assert!(args.contains(&"--args=uv,sync,--all-groups".to_string()));
        assert!(!args.iter().any(|a| a.contains("--frozen")));
    }

    #[test]
    fn pipeline_respects_an_explicit_python_version() {
        let project = PythonProject { has_lockfile: true };
        let args = dagger_pipeline_args_with_version(&project, "/host/src", "3.11");
        assert_eq!(args[2], "--address=astral/uv:python3.11-trixie-slim");
    }
}
