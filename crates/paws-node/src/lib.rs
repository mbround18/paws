//! Node/JS project detection: package manager (npm/yarn/pnpm/bun) and
//! framework (Vite/Next.js/plain), from real signals in the project — not a
//! single hardcoded `pnpm` assumption. This is what `paws ci --toolchain
//! node` uses to build the right install/build/test command sequence for
//! whatever a given repo actually uses (docs/ROADMAP.md's "Node is
//! pnpm-specific today" gap).

use paws_core::Pipeline;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    Npm,
    Yarn,
    Pnpm,
    Bun,
}

impl PackageManager {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::Yarn => "yarn",
            Self::Pnpm => "pnpm",
            Self::Bun => "bun",
        }
    }

    /// Detects the package manager for a project directory. Lockfiles are
    /// the most reliable signal (they're what actually pins dependency
    /// resolution) and are checked first; `package.json`'s `packageManager`
    /// field (the Corepack convention, e.g. `"pnpm@9.1.0"`) is the fallback
    /// when no lockfile is present yet; npm is the last-resort default for a
    /// bare `package.json` with neither.
    pub fn detect(dir: &Path) -> Result<Self> {
        if dir.join("bun.lockb").is_file() || dir.join("bun.lock").is_file() {
            return Ok(Self::Bun);
        }
        if dir.join("pnpm-lock.yaml").is_file() {
            return Ok(Self::Pnpm);
        }
        if dir.join("yarn.lock").is_file() {
            return Ok(Self::Yarn);
        }
        if dir.join("package-lock.json").is_file() {
            return Ok(Self::Npm);
        }

        let package_json_path = dir.join("package.json");
        if !package_json_path.is_file() {
            anyhow::bail!("no package.json found in {}", dir.display());
        }
        let package_json = read_package_json(&package_json_path)?;

        if let Some(spec) = package_json.get("packageManager").and_then(|v| v.as_str()) {
            let name = spec.split('@').next().unwrap_or(spec);
            return match name {
                "npm" => Ok(Self::Npm),
                "yarn" => Ok(Self::Yarn),
                "pnpm" => Ok(Self::Pnpm),
                "bun" => Ok(Self::Bun),
                other => anyhow::bail!("unrecognized packageManager field: {other}"),
            };
        }

        Ok(Self::Npm)
    }

    /// The lockfile name(s) this package manager reads. `Bun` has used both
    /// a binary (`bun.lockb`) and, more recently, a text (`bun.lock`) format
    /// across versions, so both are recognized.
    pub const fn lockfile_names(&self) -> &'static [&'static str] {
        match self {
            Self::Npm => &["package-lock.json"],
            Self::Yarn => &["yarn.lock"],
            Self::Pnpm => &["pnpm-lock.yaml"],
            Self::Bun => &["bun.lockb", "bun.lock"],
        }
    }

    /// An install command appropriate for whether a lockfile actually
    /// exists. `npm ci` (and each other package manager's "frozen"/
    /// "immutable" flag) *requires* its lockfile to already be present and
    /// errors otherwise — a repo detected via `PackageManager::detect`'s
    /// fallback paths (no lockfile yet, e.g. a fresh `package.json` with a
    /// `packageManager` field but nothing committed yet) needs the plain
    /// install variant instead, which is happy to create one.
    pub fn install_args(&self, has_lockfile: bool) -> Vec<&'static str> {
        match (self, has_lockfile) {
            (Self::Npm, true) => vec!["npm", "ci"],
            (Self::Npm, false) => vec!["npm", "install"],
            (Self::Yarn, true) => vec!["yarn", "install", "--frozen-lockfile"],
            (Self::Yarn, false) => vec!["yarn", "install"],
            (Self::Pnpm, true) => vec!["pnpm", "install", "--frozen-lockfile"],
            (Self::Pnpm, false) => vec!["pnpm", "install"],
            (Self::Bun, true) => vec!["bun", "install", "--frozen-lockfile"],
            (Self::Bun, false) => vec!["bun", "install"],
        }
    }

    pub fn run_script_args(&self, script: &str) -> Vec<String> {
        match self {
            Self::Npm => vec!["npm".to_string(), "run".to_string(), script.to_string()],
            Self::Yarn => vec!["yarn".to_string(), "run".to_string(), script.to_string()],
            Self::Pnpm => vec!["pnpm".to_string(), "run".to_string(), script.to_string()],
            Self::Bun => vec!["bun".to_string(), "run".to_string(), script.to_string()],
        }
    }

    /// Setup command to run once before `install_args`, if any. `corepack`
    /// (bundled with Node 16.9+) manages yarn/pnpm at pinned versions; bun
    /// isn't a corepack-managed package manager, so its own base image is
    /// expected to already have it (see [`base_image`]).
    pub fn setup_args(&self) -> Option<Vec<&'static str>> {
        match self {
            Self::Yarn | Self::Pnpm => Some(vec!["corepack", "enable"]),
            Self::Npm | Self::Bun => None,
        }
    }

    /// The container image whose build should run in. `npm`/`yarn`/`pnpm`
    /// all ship with the official Node image (`corepack` handles yarn/pnpm
    /// version pinning); `bun` needs its own image since it isn't bundled
    /// with Node. `node:lts-trixie` (not a pinned major like `node:24-trixie`)
    /// is Docker Hub's own self-updating alias for whichever Node major is
    /// current LTS — confirmed for real this tag exists and is maintained
    /// by the official image, so there's nothing for `paws` itself (or
    /// Renovate) to track here; it moves on its own as Node designates new
    /// LTS releases, the same way `oven/bun:1-debian`/`golang:1-bookworm`/
    /// `rust:1-bookworm` already float to "latest" for ecosystems with no
    /// LTS concept to track in the first place.
    pub const fn base_image(&self) -> &'static str {
        match self {
            Self::Npm | Self::Yarn | Self::Pnpm => "node:lts-trixie",
            Self::Bun => "oven/bun:1-debian",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framework {
    Vite,
    NextJs,
    Plain,
}

impl Framework {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Vite => "vite",
            Self::NextJs => "next.js",
            Self::Plain => "plain",
        }
    }

    /// Detects the framework from `package.json`'s dependencies (checked
    /// first — a project that depends on a framework is using it, config
    /// files can be absent or renamed) and, failing that, well-known config
    /// file names as a fallback signal.
    pub fn detect(package_json: &Value, dir: &Path) -> Self {
        let has_dep = |name: &str| -> bool {
            ["dependencies", "devDependencies"].iter().any(|section| {
                package_json
                    .get(section)
                    .and_then(|d| d.get(name))
                    .is_some()
            })
        };

        // Checked before Vite: a Next.js project commonly also depends on
        // React/other Vite-adjacent tooling, but "next" itself is the
        // unambiguous signal a plain "react" or "vite" dependency isn't.
        if has_dep("next") {
            return Self::NextJs;
        }
        if has_dep("vite") {
            return Self::Vite;
        }

        for config in ["next.config.js", "next.config.mjs", "next.config.ts"] {
            if dir.join(config).is_file() {
                return Self::NextJs;
            }
        }
        for config in ["vite.config.js", "vite.config.ts", "vite.config.mjs"] {
            if dir.join(config).is_file() {
                return Self::Vite;
            }
        }

        Self::Plain
    }
}

fn read_package_json(path: &Path) -> Result<Value> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse {} as JSON", path.display()))
}

/// A detected Node/JS project: package manager, framework, and which of the
/// scripts `paws ci` needs (`build`, `test`) are actually present.
#[derive(Debug, Clone)]
pub struct NodeProject {
    pub package_manager: PackageManager,
    pub framework: Framework,
    pub has_build_script: bool,
    pub has_test_script: bool,
    pub has_lint_script: bool,
    /// Whether `package_manager`'s own lockfile actually exists — determines
    /// which install command variant is safe to use (see
    /// `PackageManager::install_args`).
    pub has_lockfile: bool,
    /// Whether this is a Playwright e2e project (`@playwright/test` as a
    /// dependency, or a `playwright.config.*` file) — a real, genuinely
    /// different shape of Node project: a fresh `npm create playwright@latest`
    /// scaffold has an empty `scripts` object (no `build`/`test` at all),
    /// so it needs its own pipeline and its own exemption from
    /// [`NodeProject::missing_required_scripts`], the same way Tauri
    /// projects already do.
    pub has_playwright: bool,
}

impl NodeProject {
    pub fn missing_required_scripts(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if !self.has_build_script {
            missing.push("build");
        }
        if !self.has_test_script {
            missing.push("test");
        }
        missing
    }
}

/// Detects everything about a Node project in `dir` needed to run `paws ci`
/// against it: package manager, framework, and which required scripts exist.
pub fn detect_project(dir: &Path) -> Result<NodeProject> {
    let package_manager = PackageManager::detect(dir)?;
    let package_json = read_package_json(&dir.join("package.json"))?;
    let framework = Framework::detect(&package_json, dir);

    let has_script = |name: &str| -> bool {
        package_json
            .get("scripts")
            .and_then(|s| s.get(name))
            .and_then(|v| v.as_str())
            .is_some()
    };

    let has_lockfile = package_manager
        .lockfile_names()
        .iter()
        .any(|name| dir.join(name).is_file());

    let has_playwright_dep = ["dependencies", "devDependencies"].iter().any(|section| {
        package_json
            .get(section)
            .and_then(|d| d.get("@playwright/test"))
            .is_some()
    });
    let has_playwright_config = [
        "playwright.config.ts",
        "playwright.config.js",
        "playwright.config.mjs",
        "playwright.config.cjs",
    ]
    .iter()
    .any(|config| dir.join(config).is_file());

    Ok(NodeProject {
        package_manager,
        framework,
        has_build_script: has_script("build"),
        has_test_script: has_script("test"),
        has_lint_script: has_script("lint"),
        has_lockfile,
        has_playwright: has_playwright_dep || has_playwright_config,
    })
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`)
/// that installs, builds, tests, and (if present) lints `project` inside a
/// container based on its package manager's `base_image` — then, if
/// `project.has_playwright`, also runs `npx playwright install --with-deps`
/// and `npx playwright test`. Playwright is additive, not exclusive: a real
/// app commonly has its own unit `test`/`lint`/`build` scripts *and* a
/// Playwright e2e suite in the same `package.json`, and both need to run —
/// a pure `npm create playwright@latest` scaffold (no `build`/`test`
/// scripts at all) still works, since those steps are only added when the
/// scripts actually exist. No `xvfb` involved and none needed for the
/// Playwright step — verified directly, end to end, against a real
/// scaffold: `playwright install --with-deps` already handles every system
/// dependency modern headless Chromium needs on its own. `xvfb` only
/// matters for `headed`/non-default display configurations, which this
/// doesn't attempt to support. Pure string construction — testable without
/// a real `dagger`/container engine.
pub fn dagger_pipeline_args(project: &NodeProject, source_dir: &str) -> Vec<String> {
    let pm = project.package_manager;
    let mut pipeline = Pipeline::from_image(pm.base_image())
        .mount("/src", source_dir)
        .workdir("/src");

    if let Some(setup) = pm.setup_args() {
        pipeline = pipeline.exec(setup);
    }
    pipeline = pipeline
        .exec(pm.install_args(project.has_lockfile))
        .exec_if(project.has_build_script, pm.run_script_args("build"))
        .exec_if(project.has_test_script, pm.run_script_args("test"))
        .exec_if(project.has_lint_script, pm.run_script_args("lint"));

    if project.has_playwright {
        pipeline = pipeline
            .exec(["npx", "playwright", "install", "--with-deps"])
            .exec(["npx", "playwright", "test"]);
    }

    pipeline.stdout()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        paws_core::test_support::scratch_dir("node", name)
    }

    #[test]
    fn detects_package_manager_from_lockfile() {
        for (file, expected) in [
            ("package-lock.json", PackageManager::Npm),
            ("yarn.lock", PackageManager::Yarn),
            ("pnpm-lock.yaml", PackageManager::Pnpm),
            ("bun.lockb", PackageManager::Bun),
        ] {
            let dir = temp_dir(file);
            fs::write(dir.join(file), "").unwrap();
            assert_eq!(
                PackageManager::detect(&dir).unwrap(),
                expected,
                "for {file}"
            );
            fs::remove_dir_all(&dir).unwrap();
        }
    }

    #[test]
    fn falls_back_to_package_manager_field_when_no_lockfile() {
        let dir = temp_dir("packagemanager-field");
        fs::write(
            dir.join("package.json"),
            r#"{"packageManager": "pnpm@9.1.0"}"#,
        )
        .unwrap();
        assert_eq!(PackageManager::detect(&dir).unwrap(), PackageManager::Pnpm);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn defaults_to_npm_with_bare_package_json() {
        let dir = temp_dir("bare");
        fs::write(dir.join("package.json"), r#"{"name": "bare"}"#).unwrap();
        assert_eq!(PackageManager::detect(&dir).unwrap(), PackageManager::Npm);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_when_no_package_json_at_all() {
        let dir = temp_dir("empty");
        assert!(PackageManager::detect(&dir).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lockfile_wins_over_conflicting_package_manager_field() {
        // A repo whose packageManager field is stale/wrong but has a real
        // lockfile should trust the lockfile — it's what actually pins
        // resolution.
        let dir = temp_dir("conflict");
        fs::write(dir.join("yarn.lock"), "").unwrap();
        fs::write(
            dir.join("package.json"),
            r#"{"packageManager": "pnpm@9.1.0"}"#,
        )
        .unwrap();
        assert_eq!(PackageManager::detect(&dir).unwrap(), PackageManager::Yarn);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_nextjs_from_dependency() {
        let json: Value = serde_json::from_str(r#"{"dependencies": {"next": "^14.0.0"}}"#).unwrap();
        let dir = temp_dir("next-dep");
        assert_eq!(Framework::detect(&json, &dir), Framework::NextJs);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_vite_from_dev_dependency() {
        let json: Value =
            serde_json::from_str(r#"{"devDependencies": {"vite": "^5.0.0"}}"#).unwrap();
        let dir = temp_dir("vite-dep");
        assert_eq!(Framework::detect(&json, &dir), Framework::Vite);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_vite_from_config_file_when_no_dependency_listed() {
        let json: Value = serde_json::from_str(r"{}").unwrap();
        let dir = temp_dir("vite-config");
        fs::write(dir.join("vite.config.ts"), "export default {}").unwrap();
        assert_eq!(Framework::detect(&json, &dir), Framework::Vite);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn plain_project_has_no_framework() {
        let json: Value =
            serde_json::from_str(r#"{"dependencies": {"express": "^4.0.0"}}"#).unwrap();
        let dir = temp_dir("plain");
        assert_eq!(Framework::detect(&json, &dir), Framework::Plain);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_required_scripts_reports_both_when_absent() {
        let project = NodeProject {
            package_manager: PackageManager::Npm,
            framework: Framework::Plain,
            has_build_script: false,
            has_test_script: false,
            has_lint_script: false,
            has_lockfile: false,
            has_playwright: false,
        };
        assert_eq!(project.missing_required_scripts(), vec!["build", "test"]);
    }

    fn project_for(pm: PackageManager) -> NodeProject {
        NodeProject {
            package_manager: pm,
            framework: Framework::Plain,
            has_build_script: true,
            has_test_script: true,
            has_lint_script: false,
            has_lockfile: true,
            has_playwright: false,
        }
    }

    #[test]
    fn dagger_pipeline_args_use_the_right_base_image_and_install_command() {
        let pnpm_args = dagger_pipeline_args(&project_for(PackageManager::Pnpm), "/host/src");
        assert_eq!(pnpm_args[2], "--address=node:lts-trixie");
        assert!(pnpm_args.contains(&"--args=corepack,enable".to_string()));
        assert!(pnpm_args.contains(&"--args=pnpm,install,--frozen-lockfile".to_string()));
        assert!(pnpm_args.contains(&"--args=pnpm,run,build".to_string()));
        assert!(pnpm_args.contains(&"--args=pnpm,run,test".to_string()));
        assert_eq!(pnpm_args.last(), Some(&"stdout".to_string()));

        let bun_args = dagger_pipeline_args(&project_for(PackageManager::Bun), "/host/src");
        assert_eq!(bun_args[2], "--address=oven/bun:1-debian");
        // bun has no setup_args (no corepack step) — first with-exec is install.
        assert!(!bun_args.contains(&"--args=corepack,enable".to_string()));
        assert!(bun_args.contains(&"--args=bun,install,--frozen-lockfile".to_string()));

        let npm_args = dagger_pipeline_args(&project_for(PackageManager::Npm), "/host/src");
        assert!(npm_args.contains(&"--args=npm,ci".to_string()));

        let yarn_args = dagger_pipeline_args(&project_for(PackageManager::Yarn), "/host/src");
        assert!(yarn_args.contains(&"--args=yarn,install,--frozen-lockfile".to_string()));
    }

    #[test]
    fn dagger_pipeline_args_includes_lint_only_when_present() {
        let mut with_lint = project_for(PackageManager::Npm);
        with_lint.has_lint_script = true;
        let args = dagger_pipeline_args(&with_lint, "/host/src");
        assert!(args.contains(&"--args=npm,run,lint".to_string()));

        let without_lint = project_for(PackageManager::Npm);
        let args = dagger_pipeline_args(&without_lint, "/host/src");
        assert!(!args.contains(&"--args=npm,run,lint".to_string()));
    }

    #[test]
    fn no_lockfile_uses_the_plain_install_variant_not_the_frozen_one() {
        // `npm ci`/`yarn install --frozen-lockfile`/etc. all *require* their
        // lockfile to already exist and error otherwise — a project reached
        // via detect()'s fallback paths (no lockfile committed yet) must get
        // the plain install command instead, or every such build fails
        // immediately (this was a real bug caught by testing against
        // examples/node-fixture-with-lint-failure, which has no lockfile).
        let mut project = project_for(PackageManager::Npm);
        project.has_lockfile = false;
        let args = dagger_pipeline_args(&project, "/host/src");
        assert!(args.contains(&"--args=npm,install".to_string()));
        assert!(!args.contains(&"--args=npm,ci".to_string()));
    }

    #[test]
    fn detects_playwright_from_dev_dependency() {
        let dir = temp_dir("playwright-dep");
        fs::write(dir.join("package-lock.json"), "").unwrap();
        fs::write(
            dir.join("package.json"),
            r#"{"devDependencies": {"@playwright/test": "^1.62.1"}, "scripts": {}}"#,
        )
        .unwrap();
        let project = detect_project(&dir).unwrap();
        assert!(project.has_playwright);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_playwright_from_config_file_when_no_dependency_listed() {
        let dir = temp_dir("playwright-config");
        fs::write(dir.join("package-lock.json"), "").unwrap();
        fs::write(dir.join("package.json"), r#"{"scripts": {}}"#).unwrap();
        fs::write(dir.join("playwright.config.ts"), "").unwrap();
        let project = detect_project(&dir).unwrap();
        assert!(project.has_playwright);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn plain_project_is_not_detected_as_playwright() {
        let project = project_for(PackageManager::Npm);
        assert!(!project.has_playwright);
    }

    #[test]
    fn playwright_only_pipeline_installs_deps_and_runs_playwright_test() {
        // A fresh `npm create playwright@latest` scaffold: no build/test
        // scripts, just the e2e suite.
        let mut project = project_for(PackageManager::Npm);
        project.has_build_script = false;
        project.has_test_script = false;
        project.has_playwright = true;
        let args = dagger_pipeline_args(&project, "/host/src");
        assert_eq!(args[0], "container");
        assert_eq!(args[2], "--address=node:lts-trixie");
        assert!(args.contains(&"--args=npm,ci".to_string()));
        assert!(!args.contains(&"--args=npm,run,build".to_string()));
        assert!(!args.contains(&"--args=npm,run,test".to_string()));
        assert!(args.contains(&"--args=npx,playwright,install,--with-deps".to_string()));
        assert!(args.contains(&"--args=npx,playwright,test".to_string()));
        assert_eq!(args.last(), Some(&"stdout".to_string()));
    }

    #[test]
    fn playwright_is_additive_to_a_real_apps_own_build_test_lint_scripts() {
        // A real app with its own unit test/lint/build scripts *and* a
        // Playwright e2e suite in the same package.json — both must run,
        // not one instead of the other.
        let mut project = project_for(PackageManager::Npm);
        project.has_lint_script = true;
        project.has_playwright = true;
        let args = dagger_pipeline_args(&project, "/host/src");
        assert!(args.contains(&"--args=npm,run,build".to_string()));
        assert!(args.contains(&"--args=npm,run,test".to_string()));
        assert!(args.contains(&"--args=npm,run,lint".to_string()));
        assert!(args.contains(&"--args=npx,playwright,install,--with-deps".to_string()));
        assert!(args.contains(&"--args=npx,playwright,test".to_string()));
        // build/test/lint must run before the playwright steps.
        let build_idx = args
            .iter()
            .position(|a| a == "--args=npm,run,build")
            .unwrap();
        let playwright_idx = args
            .iter()
            .position(|a| a == "--args=npx,playwright,install,--with-deps")
            .unwrap();
        assert!(build_idx < playwright_idx);
    }
}
