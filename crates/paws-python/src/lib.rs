//! Native Python CI support, ported from `gh-reusable`'s `pythonBuildAndTest`
//! Dagger function for parity (`packages/dagger-module/src/index.ts`):
//! `uv sync --all-groups [--frozen]`, `uv build`, `uv run pytest`, against
//! the `astral/uv:python<version>-trixie-slim` image. Only `uv`-based
//! projects (a `pyproject.toml`) are supported — `gh-reusable` never had a
//! poetry/pipenv/pip path to port, and inventing one here would be exactly
//! the "reimplementation from memory" the project's parity principle exists
//! to avoid. The one deliberate divergence: the `pytest` step is conditional
//! (see [`TestPlan`]). `gh-reusable` ran it unconditionally, which turns any
//! project that doesn't use pytest into a failed build for a tool it never
//! asked for.

use paws_core::Pipeline;
use std::path::Path;

use anyhow::{Context, Result};

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
    /// Whether anything in `pyproject.toml` actually installs `pytest` —
    /// `[project] dependencies`, `[project.optional-dependencies]`, or
    /// `[dependency-groups]`. Without one of those, `uv sync` never puts a
    /// `pytest` on PATH and `uv run pytest` dies with "Failed to spawn".
    pub declares_pytest: bool,
    /// Whether the project looks like it has tests to run at all: a `tests/`
    /// directory, a root-level `test_*.py`/`*_test.py`, or pytest
    /// configuration (`[tool.pytest.ini_options]`, `pytest.ini`).
    pub has_tests: bool,
}

/// What the pipeline should do about the test step, decided before a
/// container is ever built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestPlan {
    /// `pytest` is installed by the sync — run it.
    Run,
    /// Nothing to test: no `pytest` declared and no tests to find. The
    /// pipeline builds and stops there rather than failing on a tool the
    /// project never asked for.
    Skip,
    /// Tests are here but nothing installs `pytest`. Failing this at the
    /// pipeline's last `withExec` costs a full sync+build first, so the
    /// caller errors on it up front instead.
    MissingPytest,
}

impl PythonProject {
    #[must_use]
    pub const fn test_plan(&self) -> TestPlan {
        match (self.declares_pytest, self.has_tests) {
            (true, _) => TestPlan::Run,
            (false, true) => TestPlan::MissingPytest,
            (false, false) => TestPlan::Skip,
        }
    }
}

/// Does any dependency list in `pyproject.toml` install `pytest`?
///
/// Reads the three places `uv` itself installs from, rather than grepping the
/// file: a `pytest` in `[tool.ruff]`'s config or in a comment is not a
/// dependency, and `pytest-cov` alone does install `pytest` (it depends on
/// it), which a naive exact-name match on the requirement string would miss
/// — so each requirement is compared on its parsed distribution name with a
/// `pytest`-prefix allowance for the plugin ecosystem.
fn declares_pytest(pyproject: &toml::Value) -> bool {
    fn requirement_is_pytest(requirement: &toml::Value) -> bool {
        let Some(requirement) = requirement.as_str() else {
            return false;
        };
        // "pytest>=8", "pytest-cov[toml] >= 5", "pytest ; python_version<'3.13'"
        let name = requirement
            .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_' || c == '.'))
            .find(|part| !part.is_empty())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .replace('_', "-");
        name == "pytest" || name.starts_with("pytest-")
    }

    fn any_requirement_is_pytest(list: Option<&toml::Value>) -> bool {
        list.and_then(toml::Value::as_array)
            .is_some_and(|deps| deps.iter().any(requirement_is_pytest))
    }

    fn any_group_declares_pytest(groups: Option<&toml::Value>) -> bool {
        groups
            .and_then(toml::Value::as_table)
            .is_some_and(|groups| {
                groups
                    .values()
                    .any(|list| any_requirement_is_pytest(Some(list)))
            })
    }

    let project = pyproject.get("project");
    any_requirement_is_pytest(project.and_then(|p| p.get("dependencies")))
        || any_group_declares_pytest(project.and_then(|p| p.get("optional-dependencies")))
        || any_group_declares_pytest(pyproject.get("dependency-groups"))
}

/// Does the project have tests worth running? Deliberately generous: a false
/// "yes" turns into an actionable "declare pytest" error, while a false "no"
/// would silently skip a real test suite.
fn has_tests(dir: &Path, pyproject: &toml::Value) -> bool {
    if pyproject
        .get("tool")
        .and_then(|tool| tool.get("pytest"))
        .is_some()
        || dir.join("pytest.ini").is_file()
    {
        return true;
    }
    if dir.join("tests").is_dir() || dir.join("test").is_dir() {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        name.ends_with(".py") && (name.starts_with("test_") || name.ends_with("_test.py"))
    })
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
    let pyproject_path = dir.join("pyproject.toml");
    let text = std::fs::read_to_string(&pyproject_path)
        .with_context(|| format!("failed to read {}", pyproject_path.display()))?;
    let pyproject: toml::Value = toml::from_str(&text)
        .with_context(|| format!("failed to parse {}", pyproject_path.display()))?;

    Ok(PythonProject {
        has_lockfile: dir.join("uv.lock").is_file(),
        declares_pytest: declares_pytest(&pyproject),
        has_tests: has_tests(dir, &pyproject),
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

    let mut pipeline = Pipeline::from_image(image)
        .mount("/src", source_dir)
        .workdir("/src")
        .exec(sync)
        .exec(["uv", "build"]);
    // A project that declares no pytest gets no pytest step: `uv run pytest`
    // would fail with "Failed to spawn: `pytest`" at the end of a full sync
    // and build, which says nothing useful about the project.
    if project.test_plan() == TestPlan::Run {
        pipeline = pipeline.exec(["uv", "run", "pytest"]);
    }
    pipeline.stdout()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A project whose only interesting axis for pipeline tests is the
    /// lockfile — it declares pytest, so the test step is present.
    fn testable(has_lockfile: bool) -> PythonProject {
        PythonProject {
            has_lockfile,
            declares_pytest: true,
            has_tests: true,
        }
    }

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
        let project = testable(true);
        let args = dagger_pipeline_args(&project, "/host/src");
        assert_eq!(args[0], "container");
        assert_eq!(args[1], "from");
        assert_eq!(args[2], "--address=astral/uv:python3.13-trixie-slim");
    }

    #[test]
    fn pipeline_runs_frozen_sync_when_a_lockfile_exists() {
        let project = testable(true);
        let args = dagger_pipeline_args(&project, "/host/src");
        assert!(args.contains(&"--args=uv,sync,--all-groups,--frozen".to_string()));
        assert!(args.contains(&"--args=uv,build".to_string()));
        assert!(args.contains(&"--args=uv,run,pytest".to_string()));
        assert_eq!(args.last(), Some(&"stdout".to_string()));
    }

    #[test]
    fn pipeline_omits_frozen_flag_without_a_lockfile() {
        let project = testable(false);
        let args = dagger_pipeline_args(&project, "/host/src");
        assert!(args.contains(&"--args=uv,sync,--all-groups".to_string()));
        assert!(!args.iter().any(|a| a.contains("--frozen")));
    }

    #[test]
    fn pytest_is_detected_across_every_place_uv_installs_from() {
        let cases = [
            ("[project]\ndependencies = [\"pytest>=8\"]\n", true),
            (
                "[project]\ndependencies = []\n[project.optional-dependencies]\ndev = [\"pytest\"]\n",
                true,
            ),
            (
                "[dependency-groups]\ndev = [\"pytest-cov[toml] >= 5\"]\n",
                true,
            ),
            (
                "[dependency-groups]\ndev = [\"pytest ; python_version<'3.14'\"]\n",
                true,
            ),
            // `pytest` named anywhere that doesn't install it doesn't count.
            ("[tool.ruff]\nextend-exclude = [\"pytest\"]\n", false),
            ("[project]\ndependencies = [\"requests\"]\n", false),
        ];

        for (pyproject, expected) in cases {
            let parsed: toml::Value = toml::from_str(pyproject).unwrap();
            assert_eq!(
                declares_pytest(&parsed),
                expected,
                "wrong answer for: {pyproject}"
            );
        }
    }

    #[test]
    fn a_project_with_no_pytest_and_no_tests_skips_the_test_step() {
        // The hammock case: a pyproject with real dependencies, no dev group,
        // and no tests. Building it must not fail on a missing `pytest`.
        let dir = temp_dir("no-tests");
        fs::write(
            dir.join("pyproject.toml"),
            "[project]\nname = \"x\"\ndependencies = [\"openai-whisper\"]\n",
        )
        .unwrap();

        let project = detect_project(&dir).unwrap();
        assert_eq!(project.test_plan(), TestPlan::Skip);

        let args = dagger_pipeline_args(&project, "/host/src");
        assert!(args.contains(&"--args=uv,build".to_string()));
        assert!(
            !args.iter().any(|a| a.contains("pytest")),
            "no pytest step should be generated: {args:?}"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tests_without_a_declared_pytest_are_flagged_not_silently_skipped() {
        let dir = temp_dir("tests-no-pytest");
        fs::write(dir.join("pyproject.toml"), "[project]\nname = \"x\"\n").unwrap();
        fs::create_dir_all(dir.join("tests")).unwrap();

        assert_eq!(
            detect_project(&dir).unwrap().test_plan(),
            TestPlan::MissingPytest
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_declared_pytest_still_runs() {
        let dir = temp_dir("declared-pytest");
        fs::write(
            dir.join("pyproject.toml"),
            "[project]\nname = \"x\"\n[dependency-groups]\ndev = [\"pytest\"]\n",
        )
        .unwrap();

        let project = detect_project(&dir).unwrap();
        assert_eq!(project.test_plan(), TestPlan::Run);
        assert!(
            dagger_pipeline_args(&project, "/host/src")
                .contains(&"--args=uv,run,pytest".to_string())
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pytest_config_alone_counts_as_having_tests() {
        let dir = temp_dir("pytest-config");
        fs::write(
            dir.join("pyproject.toml"),
            "[project]\nname = \"x\"\n[tool.pytest.ini_options]\naddopts = \"-q\"\n",
        )
        .unwrap();

        assert_eq!(
            detect_project(&dir).unwrap().test_plan(),
            TestPlan::MissingPytest
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pipeline_respects_an_explicit_python_version() {
        let project = testable(true);
        let args = dagger_pipeline_args_with_version(&project, "/host/src", "3.11");
        assert_eq!(args[2], "--address=astral/uv:python3.11-trixie-slim");
    }
}
