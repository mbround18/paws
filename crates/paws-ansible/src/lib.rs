//! Native Ansible CI support: `ansible-galaxy install -r requirements.yml`,
//! `ansible-lint`, and `ansible-playbook --syntax-check` over every playbook
//! the repo ships, against the same `astral/uv:python<version>-trixie-slim`
//! image `paws-python` builds on.
//!
//! An Ansible control repo is a Python project wearing a hat — its
//! `ansible-core`, `ansible-lint` and `molecule` versions are pinned through
//! normal Python packaging — so this crate reuses the Python image and `uv`
//! rather than inventing an Ansible-specific runtime. What it does *not* do
//! is run Molecule: every Molecule scenario worth having drives a real
//! container per role, and a Dagger container has no Docker daemon to give
//! it. Scenarios are detected and reported (see
//! [`AnsibleProject::molecule_scenarios`]) so CI can run them on a
//! Docker-enabled runner as a separate job, instead of being silently
//! skipped here and looking green.

use paws_core::Pipeline;
use std::path::Path;

use anyhow::{Context, Result};

/// Matches `paws-python`'s default: this pipeline runs on the same
/// `astral/uv` image, and an Ansible repo's Python version is resolved from
/// the same `.python-version`/`.tool-versions` files.
pub const DEFAULT_PYTHON_VERSION: &str = "3.13";

fn base_image(python_version: &str) -> String {
    format!("astral/uv:python{python_version}-trixie-slim")
}

/// How the repo installs Ansible itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Packaging {
    /// A `pyproject.toml` project: `uv sync --all-groups` installs
    /// `ansible-core`/`ansible-lint` at the versions the repo pinned, and
    /// every tool runs under `uv run`.
    Uv {
        /// Whether `uv.lock` is committed — `--frozen` requires it, exactly
        /// as in `paws-python`.
        has_lockfile: bool,
    },
    /// A bare collection or role repo with no Python packaging of its own
    /// (the shape Galaxy publishes). Nothing pins a version, so the pipeline
    /// installs `ansible-core` and `ansible-lint` into the image directly.
    Bare,
}

#[derive(Debug, Clone)]
pub struct AnsibleProject {
    pub packaging: Packaging,
    /// Whether anything in `pyproject.toml` installs `ansible-lint`. A `uv`
    /// project that doesn't is an error rather than a skipped step: lint is
    /// the whole point of this pipeline, and `uv run ansible-lint` would die
    /// with "Failed to spawn" after a full sync.
    pub declares_ansible_lint: bool,
    /// A `requirements.yml` naming the collections and roles the repo
    /// depends on. Without one, a playbook using `community.general.*`
    /// syntax-checks fine locally (where the collection happens to be
    /// installed) and fails in a clean container — so its presence decides
    /// whether the galaxy step runs, and its absence is reported.
    pub requirements_file: Option<String>,
    /// Playbooks to syntax-check, relative to the repo root, sorted.
    pub playbooks: Vec<String>,
    /// Roles with a `molecule/` directory. Reported, never run — see the
    /// module docs.
    pub molecule_scenarios: Vec<String>,
}

/// What the pipeline should do about the lint step, decided before a
/// container is ever built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LintPlan {
    /// `ansible-lint` is installed by the sync (or by the bare install) —
    /// run it.
    Run,
    /// A `uv` project that pins no `ansible-lint`. The caller errors on this
    /// up front rather than paying for a full sync first.
    MissingAnsibleLint,
}

impl AnsibleProject {
    #[must_use]
    pub const fn lint_plan(&self) -> LintPlan {
        match self.packaging {
            // Nothing to pin against, so the pipeline installs it itself.
            Packaging::Bare => LintPlan::Run,
            Packaging::Uv { .. } if self.declares_ansible_lint => LintPlan::Run,
            Packaging::Uv { .. } => LintPlan::MissingAnsibleLint,
        }
    }
}

/// Does any dependency list in `pyproject.toml` install `ansible-lint`?
///
/// Reads the three places `uv` installs from rather than grepping, for the
/// same reason `paws-python` does: `ansible-lint` named in a `[tool.*]`
/// section or a comment is configuration, not a dependency.
fn declares_ansible_lint(pyproject: &toml::Value) -> bool {
    fn requirement_is_ansible_lint(requirement: &toml::Value) -> bool {
        let Some(requirement) = requirement.as_str() else {
            return false;
        };
        // "ansible-lint>=25", "ansible_lint ; python_version>='3.11'"
        let name = requirement
            .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_' || c == '.'))
            .find(|part| !part.is_empty())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .replace('_', "-");
        name == "ansible-lint"
    }

    fn any_requirement_matches(list: Option<&toml::Value>) -> bool {
        list.and_then(toml::Value::as_array)
            .is_some_and(|deps| deps.iter().any(requirement_is_ansible_lint))
    }

    fn any_group_matches(groups: Option<&toml::Value>) -> bool {
        groups
            .and_then(toml::Value::as_table)
            .is_some_and(|groups| {
                groups
                    .values()
                    .any(|list| any_requirement_matches(Some(list)))
            })
    }

    let project = pyproject.get("project");
    any_requirement_matches(project.and_then(|p| p.get("dependencies")))
        || any_group_matches(project.and_then(|p| p.get("optional-dependencies")))
        || any_group_matches(pyproject.get("dependency-groups"))
}

fn is_yaml(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("yml") || extension.eq_ignore_ascii_case("yaml")
        })
}

/// Playbooks to syntax-check.
///
/// Every YAML file directly under `playbooks/` when that directory exists —
/// the layout `ansible-lint`'s own docs and every repo of this shape use —
/// and otherwise the conventional root-level entrypoint names. Deliberately
/// not "every YAML file in the repo": role task files, `group_vars`, and
/// `molecule.yml` are all YAML and none of them are playbooks, and
/// `--syntax-check` on one fails with a confusing error about a missing
/// `hosts` key.
fn find_playbooks(dir: &Path) -> Vec<String> {
    let playbook_dir = dir.join("playbooks");
    if playbook_dir.is_dir() {
        let Ok(entries) = std::fs::read_dir(&playbook_dir) else {
            return Vec::new();
        };
        let mut found: Vec<String> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_file() && is_yaml(path))
            .filter_map(|path| {
                path.file_name()
                    .map(|name| format!("playbooks/{}", name.to_string_lossy()))
            })
            .collect();
        found.sort();
        return found;
    }
    [
        "site.yml",
        "site.yaml",
        "playbook.yml",
        "playbook.yaml",
        "main.yml",
    ]
    .iter()
    .filter(|name| dir.join(name).is_file())
    .map(|name| (*name).to_string())
    .collect()
}

/// Roles under `roles/` that ship a Molecule scenario.
fn find_molecule_scenarios(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir.join("roles")) else {
        return Vec::new();
    };
    let mut found: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().join("molecule").is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

fn find_requirements_file(dir: &Path) -> Option<String> {
    [
        "requirements.yml",
        "requirements.yaml",
        "collections/requirements.yml",
    ]
    .into_iter()
    .find(|name| dir.join(name).is_file())
    .map(str::to_string)
}

/// An Ansible repo has an `ansible.cfg`, playbooks, or roles with real task
/// files. Any one of the three is enough: a control repo usually has all
/// three, a collection of roles has only the last.
pub fn is_ansible_project(dir: &Path) -> bool {
    if dir.join("ansible.cfg").is_file() {
        return true;
    }
    if !find_playbooks(dir).is_empty() {
        return true;
    }
    std::fs::read_dir(dir.join("roles")).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.path().join("tasks").join("main.yml").is_file())
    })
}

pub fn detect_project(dir: &Path) -> Result<AnsibleProject> {
    if !is_ansible_project(dir) {
        anyhow::bail!(
            "no ansible.cfg, playbooks/, or roles/*/tasks/main.yml found in {}",
            dir.display()
        );
    }

    let pyproject_path = dir.join("pyproject.toml");
    let (packaging, declares_ansible_lint) = if pyproject_path.is_file() {
        let text = std::fs::read_to_string(&pyproject_path)
            .with_context(|| format!("failed to read {}", pyproject_path.display()))?;
        let pyproject: toml::Value = toml::from_str(&text)
            .with_context(|| format!("failed to parse {}", pyproject_path.display()))?;
        (
            Packaging::Uv {
                has_lockfile: dir.join("uv.lock").is_file(),
            },
            declares_ansible_lint(&pyproject),
        )
    } else {
        (Packaging::Bare, false)
    };

    Ok(AnsibleProject {
        packaging,
        declares_ansible_lint,
        requirements_file: find_requirements_file(dir),
        playbooks: find_playbooks(dir),
        molecule_scenarios: find_molecule_scenarios(dir),
    })
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`)
/// for `project`, using [`DEFAULT_PYTHON_VERSION`].
#[must_use]
pub fn dagger_pipeline_args(project: &AnsibleProject, source_dir: &str) -> Vec<String> {
    dagger_pipeline_args_with_version(project, source_dir, DEFAULT_PYTHON_VERSION)
}

/// Same as [`dagger_pipeline_args`], with an explicit Python version.
#[must_use]
pub fn dagger_pipeline_args_with_version(
    project: &AnsibleProject,
    source_dir: &str,
    python_version: &str,
) -> Vec<String> {
    dagger_pipeline_args_with_image(project, source_dir, &base_image(python_version))
}

/// [`dagger_pipeline_args`] against an explicit image — see
/// `paws_core::Toolchain::image_for`.
#[must_use]
pub fn dagger_pipeline_args_with_image(
    project: &AnsibleProject,
    source_dir: &str,
    image: &str,
) -> Vec<String> {
    let mut pipeline = Pipeline::from_image(image)
        .mount("/src", source_dir)
        .workdir("/src");

    // `uv run` in front of every tool for a uv project, nothing in front of
    // them for a bare one — the only difference between the two paths.
    let run_prefix: &[&str] = match project.packaging {
        Packaging::Uv { has_lockfile } => {
            let mut sync = vec!["uv", "sync", "--all-groups"];
            if has_lockfile {
                sync.push("--frozen");
            }
            pipeline = pipeline.exec(sync);
            &["uv", "run"]
        }
        Packaging::Bare => {
            pipeline = pipeline.exec([
                "uv",
                "pip",
                "install",
                "--system",
                "ansible-core",
                "ansible-lint",
            ]);
            &[]
        }
    };
    let run = |command: &[&str]| -> Vec<String> {
        run_prefix
            .iter()
            .chain(command.iter())
            .map(|part| (*part).to_string())
            .collect()
    };

    // Collections first: `ansible-lint` and `--syntax-check` both resolve
    // module names, and both fail on a `community.general.*` task in a
    // container where nothing installed that collection.
    if let Some(requirements) = &project.requirements_file {
        pipeline = pipeline.exec(run(&["ansible-galaxy", "install", "-r", requirements]));
    }

    if project.lint_plan() == LintPlan::Run {
        pipeline = pipeline.exec(run(&["ansible-lint"]));
    }

    // One `--syntax-check` invocation per playbook: passing them all to one
    // command works, but a failure then names only the first, and the point
    // of this step is knowing *which* playbook broke.
    for playbook in &project.playbooks {
        pipeline = pipeline.exec(run(&["ansible-playbook", "--syntax-check", playbook]));
    }

    pipeline.stdout()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        paws_core::test_support::scratch_dir("ansible", name)
    }

    /// A control repo of the shape this toolchain targets: uv-managed, with
    /// a lockfile, a lint pin, playbooks, and roles.
    fn control_repo(dir: &Path) {
        fs::write(dir.join("ansible.cfg"), "[defaults]\nroles_path = roles\n").unwrap();
        fs::write(
            dir.join("pyproject.toml"),
            "[project]\nname = \"x\"\ndependencies = [\"ansible-core\"]\n\
             [dependency-groups]\ndev = [\"ansible-lint\"]\n",
        )
        .unwrap();
        fs::write(dir.join("uv.lock"), "").unwrap();
        fs::create_dir_all(dir.join("playbooks")).unwrap();
        fs::write(dir.join("playbooks/site.yaml"), "---\n- hosts: all\n").unwrap();
    }

    fn project_at(dir: &Path) -> AnsibleProject {
        detect_project(dir).unwrap()
    }

    #[test]
    fn detects_a_repo_by_ansible_cfg_playbooks_or_roles() {
        let dir = temp_dir("detect");
        assert!(!is_ansible_project(&dir), "an empty dir is not ansible");

        fs::write(dir.join("ansible.cfg"), "[defaults]\n").unwrap();
        assert!(is_ansible_project(&dir), "ansible.cfg is enough");
        fs::remove_file(dir.join("ansible.cfg")).unwrap();

        fs::create_dir_all(dir.join("playbooks")).unwrap();
        assert!(
            !is_ansible_project(&dir),
            "an empty playbooks/ dir names no playbook"
        );
        fs::write(dir.join("playbooks/site.yaml"), "---\n").unwrap();
        assert!(is_ansible_project(&dir), "a playbook is enough");
        fs::remove_dir_all(dir.join("playbooks")).unwrap();

        fs::create_dir_all(dir.join("roles/base/tasks")).unwrap();
        fs::write(dir.join("roles/base/tasks/main.yml"), "---\n").unwrap();
        assert!(is_ansible_project(&dir), "a role with tasks is enough");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_when_the_directory_is_not_an_ansible_repo() {
        let dir = temp_dir("not-ansible");
        fs::write(dir.join("pyproject.toml"), "[project]\nname = \"x\"\n").unwrap();
        assert!(detect_project(&dir).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_uv_repo_syncs_frozen_lints_and_syntax_checks_every_playbook() {
        let dir = temp_dir("uv-repo");
        control_repo(&dir);
        fs::write(dir.join("playbooks/provision.yaml"), "---\n").unwrap();
        // Not a playbook, and not YAML — neither should reach --syntax-check.
        fs::write(dir.join("playbooks/README.md"), "").unwrap();

        let project = project_at(&dir);
        assert_eq!(
            project.playbooks,
            vec!["playbooks/provision.yaml", "playbooks/site.yaml"]
        );

        let args = dagger_pipeline_args(&project, "/host/src");
        assert_eq!(args[2], "--address=astral/uv:python3.13-trixie-slim");
        assert!(args.contains(&"--args=uv,sync,--all-groups,--frozen".to_string()));
        assert!(args.contains(&"--args=uv,run,ansible-lint".to_string()));
        assert!(args.contains(
            &"--args=uv,run,ansible-playbook,--syntax-check,playbooks/provision.yaml".to_string()
        ));
        assert!(args.contains(
            &"--args=uv,run,ansible-playbook,--syntax-check,playbooks/site.yaml".to_string()
        ));
        assert_eq!(args.last(), Some(&"stdout".to_string()));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_uv_repo_without_a_lockfile_syncs_unfrozen() {
        let dir = temp_dir("no-lockfile");
        control_repo(&dir);
        fs::remove_file(dir.join("uv.lock")).unwrap();

        let args = dagger_pipeline_args(&project_at(&dir), "/host/src");
        assert!(args.contains(&"--args=uv,sync,--all-groups".to_string()));
        assert!(!args.iter().any(|arg| arg.contains("--frozen")));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_uv_repo_that_pins_no_ansible_lint_is_flagged_not_silently_skipped() {
        let dir = temp_dir("no-lint-pin");
        control_repo(&dir);
        fs::write(dir.join("pyproject.toml"), "[project]\nname = \"x\"\n").unwrap();

        let project = project_at(&dir);
        assert_eq!(project.lint_plan(), LintPlan::MissingAnsibleLint);
        assert!(
            !dagger_pipeline_args(&project, "/host/src")
                .iter()
                .any(|arg| arg.contains("ansible-lint"))
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ansible_lint_is_detected_across_every_place_uv_installs_from() {
        let cases = [
            ("[project]\ndependencies = [\"ansible-lint>=25\"]\n", true),
            (
                "[project]\ndependencies = []\n[project.optional-dependencies]\ndev = [\"ansible_lint\"]\n",
                true,
            ),
            (
                "[dependency-groups]\ndev = [\"ansible-lint ; python_version>='3.11'\"]\n",
                true,
            ),
            // `ansible-core` alone doesn't bring a linter with it.
            ("[project]\ndependencies = [\"ansible-core\"]\n", false),
            ("[tool.ruff]\nextend-exclude = [\"ansible-lint\"]\n", false),
        ];

        for (pyproject, expected) in cases {
            let parsed: toml::Value = toml::from_str(pyproject).unwrap();
            assert_eq!(
                declares_ansible_lint(&parsed),
                expected,
                "wrong answer for: {pyproject}"
            );
        }
    }

    #[test]
    fn a_bare_role_repo_installs_ansible_itself_and_runs_the_tools_directly() {
        let dir = temp_dir("bare-repo");
        fs::create_dir_all(dir.join("roles/base/tasks")).unwrap();
        fs::write(dir.join("roles/base/tasks/main.yml"), "---\n").unwrap();

        let project = project_at(&dir);
        assert_eq!(project.packaging, Packaging::Bare);
        assert_eq!(project.lint_plan(), LintPlan::Run);

        let args = dagger_pipeline_args(&project, "/host/src");
        assert!(
            args.contains(&"--args=uv,pip,install,--system,ansible-core,ansible-lint".to_string())
        );
        assert!(args.contains(&"--args=ansible-lint".to_string()));
        assert!(
            !args.iter().any(|arg| arg.contains("uv,run")),
            "a bare repo has no uv environment to run through: {args:?}"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn collections_are_installed_before_anything_resolves_a_module_name() {
        let dir = temp_dir("requirements");
        control_repo(&dir);
        fs::write(dir.join("requirements.yml"), "collections: []\n").unwrap();

        let project = project_at(&dir);
        assert_eq!(
            project.requirements_file.as_deref(),
            Some("requirements.yml")
        );

        let args = dagger_pipeline_args(&project, "/host/src");
        let galaxy = args
            .iter()
            .position(|arg| arg.contains("ansible-galaxy"))
            .expect("galaxy step present");
        let lint = args
            .iter()
            .position(|arg| arg.contains("ansible-lint"))
            .expect("lint step present");
        assert!(galaxy < lint, "galaxy must run before lint: {args:?}");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_repo_with_no_requirements_file_skips_the_galaxy_step() {
        let dir = temp_dir("no-requirements");
        control_repo(&dir);

        let project = project_at(&dir);
        assert_eq!(project.requirements_file, None);
        assert!(
            !dagger_pipeline_args(&project, "/host/src")
                .iter()
                .any(|arg| arg.contains("ansible-galaxy"))
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn molecule_scenarios_are_reported_but_never_run() {
        let dir = temp_dir("molecule");
        control_repo(&dir);
        for role in ["storage", "base"] {
            fs::create_dir_all(dir.join(format!("roles/{role}/molecule/default"))).unwrap();
        }
        fs::create_dir_all(dir.join("roles/no-scenario/tasks")).unwrap();

        let project = project_at(&dir);
        assert_eq!(project.molecule_scenarios, vec!["base", "storage"]);
        assert!(
            !dagger_pipeline_args(&project, "/host/src")
                .iter()
                .any(|arg| arg.contains("molecule")),
            "molecule needs a docker daemon a dagger container doesn't have"
        );

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn root_level_entrypoint_playbooks_are_found_without_a_playbooks_dir() {
        let dir = temp_dir("root-playbook");
        fs::write(dir.join("ansible.cfg"), "[defaults]\n").unwrap();
        fs::write(dir.join("site.yml"), "---\n- hosts: all\n").unwrap();

        assert_eq!(project_at(&dir).playbooks, vec!["site.yml"]);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pipeline_respects_an_explicit_python_version() {
        let dir = temp_dir("explicit-version");
        control_repo(&dir);
        let args = dagger_pipeline_args_with_version(&project_at(&dir), "/host/src", "3.14");
        assert_eq!(args[2], "--address=astral/uv:python3.14-trixie-slim");
        fs::remove_dir_all(&dir).unwrap();
    }
}
