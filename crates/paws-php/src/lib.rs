//! Native PHP CI support for `paws ci --toolchain php`. No `gh-reusable`
//! precedent at all here — its Dagger module never had a PHP function of
//! any kind (not even a `setupPhp`, unlike Ruby/Go/Java; see
//! `packages/dagger-module/src/index.ts`), so this is a new native
//! implementation following ordinary Composer conventions rather than a
//! port.
//!
//! Composer-only, deliberately: `composer install` + `vendor/bin/phpunit`
//! is what a real PHP project (Laravel, Symfony, a plain library) runs, and
//! a project with no `composer.json` has no dependency or autoload story
//! for this pipeline to work with.
//!
//! The base image is `composer:2` rather than the official `php:*-cli` —
//! `php:*-cli` ships no Composer at all, so using it would mean this crate
//! installing Composer itself (downloading and verifying `installer.php`),
//! reimplementing exactly what an official image already does. `composer:2`
//! is Composer's own published image and carries a full PHP CLI (8.x)
//! alongside it, so it's the single public image that already has
//! everything — no `builders/*` image needed (see `docs/ROADMAP.md`'s "How
//! a new stack gets added").

use paws_core::Pipeline;
use std::path::Path;

use anyhow::Result;

/// Composer publishes a floating major tag that tracks the current Composer
/// 2.x release (and, with it, a current PHP 8.x), so this self-updates the
/// way `node:lts-trixie`/`golang:1-bookworm` do — nothing to bump, not a
/// Renovate target (see `docs/ROADMAP.md`'s "Base image version policy").
/// Major 2 is pinned, not floated to `composer:latest`, because Composer 1
/// and 2 are genuinely incompatible resolvers and a future 3 would be too.
pub const BASE_IMAGE: &str = "composer:2";

#[derive(Debug)]
pub struct PhpProject {
    /// Whether a `PHPUnit` configuration (`phpunit.xml` or the conventional
    /// `phpunit.xml.dist` an installable package ships) is committed. This
    /// is what decides whether a test step runs at all: without a config
    /// there's no test suite for `vendor/bin/phpunit` to discover, and it
    /// would fail on the missing binary anyway when `PHPUnit` isn't a
    /// dev-dependency.
    pub has_phpunit: bool,
}

/// A Composer project has a `composer.json` at its root — the file
/// `composer install`/`composer validate` both require.
pub fn is_php_project(dir: &Path) -> bool {
    dir.join("composer.json").is_file()
}

pub fn detect_project(dir: &Path) -> Result<PhpProject> {
    if !is_php_project(dir) {
        anyhow::bail!("no composer.json found in {}", dir.display());
    }
    Ok(PhpProject {
        has_phpunit: dir.join("phpunit.xml").is_file() || dir.join("phpunit.xml.dist").is_file(),
    })
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`)
/// for `project`: `composer validate` (a real gate — it fails on a
/// malformed manifest or a `composer.lock` out of sync with
/// `composer.json`, the PHP equivalent of the lockfile-drift check
/// `paws-python`'s `--frozen` and `paws-ruby`'s `BUNDLE_FROZEN` perform),
/// `composer install`, then `PHPUnit` when the project has a suite.
///
/// `--no-check-publish` is passed to `validate` so a private/unpublished
/// project (no `description`/`license`, or a non-publishable `name`) isn't
/// failed for a packaging concern that has nothing to do with whether its
/// build is correct.
pub fn dagger_pipeline_args(project: &PhpProject, source_dir: &str) -> Vec<String> {
    dagger_pipeline_args_with_image(project, source_dir, BASE_IMAGE)
}

/// [`dagger_pipeline_args`] against an explicit image — see
/// `paws_core::Toolchain::image_for`.
pub fn dagger_pipeline_args_with_image(
    project: &PhpProject,
    source_dir: &str,
    image: &str,
) -> Vec<String> {
    Pipeline::from_image(image)
        .mount("/src", source_dir)
        .workdir("/src")
        .exec(["composer", "validate", "--strict", "--no-check-publish"])
        .exec([
            "composer",
            "install",
            "--no-interaction",
            "--prefer-dist",
            "--no-progress",
        ])
        .exec_if(project.has_phpunit, ["vendor/bin/phpunit"])
        .stdout()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        paws_core::test_support::scratch_dir("php", name)
    }

    #[test]
    fn detects_php_project_from_composer_json() {
        let dir = temp_dir("detect");
        assert!(!is_php_project(&dir));
        fs::write(dir.join("composer.json"), "{}").unwrap();
        assert!(is_php_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_when_no_composer_json() {
        let dir = temp_dir("no-composer");
        assert!(detect_project(&dir).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_phpunit_from_either_config_filename() {
        let dir = temp_dir("phpunit");
        fs::write(dir.join("composer.json"), "{}").unwrap();
        assert!(!detect_project(&dir).unwrap().has_phpunit);
        fs::write(dir.join("phpunit.xml.dist"), "<phpunit/>").unwrap();
        assert!(detect_project(&dir).unwrap().has_phpunit);
        fs::remove_dir_all(&dir).unwrap();

        let dir = temp_dir("phpunit-plain");
        fs::write(dir.join("composer.json"), "{}").unwrap();
        fs::write(dir.join("phpunit.xml"), "<phpunit/>").unwrap();
        assert!(detect_project(&dir).unwrap().has_phpunit);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pipeline_validates_then_installs_then_tests() {
        let project = PhpProject { has_phpunit: true };
        let args = dagger_pipeline_args(&project, "/host/src");
        assert_eq!(args[0], "container");
        assert_eq!(args[2], "--address=composer:2");
        assert!(args.contains(&"--args=composer,validate,--strict,--no-check-publish".to_string()));
        assert!(args.contains(
            &"--args=composer,install,--no-interaction,--prefer-dist,--no-progress".to_string()
        ));
        assert!(args.contains(&"--args=vendor/bin/phpunit".to_string()));
        assert_eq!(args.last(), Some(&"stdout".to_string()));
    }

    #[test]
    fn pipeline_skips_the_test_step_without_a_phpunit_config() {
        let project = PhpProject { has_phpunit: false };
        let args = dagger_pipeline_args(&project, "/host/src");
        assert!(!args.iter().any(|a| a.contains("phpunit")));
        assert!(args.contains(
            &"--args=composer,install,--no-interaction,--prefer-dist,--no-progress".to_string()
        ));
    }
}
