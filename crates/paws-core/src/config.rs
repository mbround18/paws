//! `paws.toml` — repo-wide configuration, loaded from the project directory.
//!
//! This is where [`PipelineDefaults`] finally comes from. That type has been
//! defined (and round-trip tested) since the first spec without anything ever
//! constructing it from a file, so every default it names was in practice
//! hard-coded at the call site instead.
//!
//! The file is entirely optional. A repo with no `paws.toml` behaves exactly
//! as it did before this existed: version files and built-in defaults decide
//! everything.
//!
//! ```toml
//! # Pin toolchains that have no native version file, or override one.
//! [toolchains]
//! ruby = "3.3.0"
//! dotnet = "9.0"
//!
//! # Pin the tools paws itself installs or runs on your behalf.
//! [tools]
//! dagger = "0.18.10"
//! semgrep = "1.81.0"
//!
//! [defaults]
//! registry = "ghcr.io"
//! changelog_path = "CHANGELOG.md"
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::PipelineDefaults;

/// The file name, at the project root.
pub const CONFIG_FILE: &str = "paws.toml";

/// A parsed `paws.toml`. Every section is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PawsConfig {
    /// Version pins per `paws ci --toolchain` value, e.g. `ruby = "3.3.0"`.
    ///
    /// Outranked by the ecosystem's own version file — see [`crate::version`].
    #[serde(default)]
    pub toolchains: BTreeMap<String, String>,
    /// Version pins for the tools `paws` installs or runs itself: the
    /// `dagger` CLI, and the scanner images `paws audit` uses.
    #[serde(default)]
    pub tools: BTreeMap<String, String>,
    /// The pre-existing [`PipelineDefaults`] contract, now actually loadable.
    #[serde(default)]
    pub defaults: PipelineDefaults,
}

impl PawsConfig {
    /// Loads `paws.toml` from `dir`, or the defaults when there is no such
    /// file.
    ///
    /// A malformed file is an error rather than a silent fallback: unlike the
    /// other tools' version files (see [`crate::version::VersionSource`]),
    /// this one is paws's own, so a typo in it is a mistake the user wants to
    /// hear about rather than have quietly ignored.
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join(CONFIG_FILE);
        let Ok(contents) = std::fs::read_to_string(&path) else {
            return Ok(Self::default());
        };
        toml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))
    }

    /// Walks up from `dir` looking for a `paws.toml`, so `paws ci --source
    /// web` in a monorepo still finds the repo-root config.
    ///
    /// The nearest file wins, which lets one package override the repo-wide
    /// pin by dropping its own `paws.toml` beside its manifest.
    pub fn discover(dir: &Path) -> Result<(Self, Option<PathBuf>)> {
        for candidate in dir.ancestors() {
            let path = candidate.join(CONFIG_FILE);
            if path.is_file() {
                return Ok((Self::load(candidate)?, Some(path)));
            }
        }
        Ok((Self::default(), None))
    }

    /// The pin for `toolchain`, if this config names one.
    pub fn toolchain_version(&self, toolchain: &str) -> Option<&str> {
        self.toolchains.get(toolchain).map(String::as_str)
    }

    /// The pin for one of paws's own tools, if this config names one.
    pub fn tool_version(&self, tool: &str) -> Option<&str> {
        self.tools.get(tool).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        crate::test_support::scratch_dir("core-config", name)
    }

    #[test]
    fn a_repo_with_no_config_gets_the_defaults() {
        let dir = scratch("absent");
        let config = PawsConfig::load(&dir).unwrap();
        assert_eq!(config, PawsConfig::default());
        assert_eq!(config.toolchain_version("rust"), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn every_section_is_optional() {
        let dir = scratch("partial");
        std::fs::write(dir.join(CONFIG_FILE), "[toolchains]\nruby = \"3.3.0\"\n").unwrap();
        let config = PawsConfig::load(&dir).unwrap();
        assert_eq!(config.toolchain_version("ruby"), Some("3.3.0"));
        assert_eq!(config.tool_version("dagger"), None);
        assert_eq!(config.defaults.registry, None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn toolchains_tools_and_defaults_all_load() {
        let dir = scratch("full");
        std::fs::write(
            dir.join(CONFIG_FILE),
            r#"
[toolchains]
ruby = "3.3.0"
dotnet = "9.0"

[tools]
dagger = "0.18.10"

[defaults]
registry = "ghcr.io"
changelog_path = "docs/CHANGELOG.md"
"#,
        )
        .unwrap();
        let config = PawsConfig::load(&dir).unwrap();
        assert_eq!(config.toolchain_version("ruby"), Some("3.3.0"));
        assert_eq!(config.toolchain_version("dotnet"), Some("9.0"));
        assert_eq!(config.tool_version("dagger"), Some("0.18.10"));
        assert_eq!(config.defaults.registry.as_deref(), Some("ghcr.io"));
        assert_eq!(
            config.defaults.changelog_path.as_deref(),
            Some("docs/CHANGELOG.md")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// paws's own config, unlike another tool's version file, must not fail
    /// quietly — a typo here is a mistake the user wants to hear about.
    #[test]
    fn a_malformed_config_is_an_error_naming_the_file() {
        let dir = scratch("malformed");
        std::fs::write(dir.join(CONFIG_FILE), "[toolchains\nruby =\n").unwrap();
        let error = PawsConfig::load(&dir).unwrap_err().to_string();
        assert!(
            error.contains(CONFIG_FILE),
            "error should name the file: {error}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_unknown_key_is_rejected_rather_than_silently_ignored() {
        let dir = scratch("unknown-key");
        std::fs::write(dir.join(CONFIG_FILE), "[toolchian]\nruby = \"3.3.0\"\n").unwrap();
        assert!(
            PawsConfig::load(&dir).is_err(),
            "a misspelled section must not be accepted as a no-op"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `paws ci --source web` in a monorepo still gets the repo-root config.
    #[test]
    fn discovery_walks_up_to_the_repo_root() {
        let root = scratch("monorepo");
        let package = root.join("web");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(root.join(CONFIG_FILE), "[toolchains]\nnode = \"22\"\n").unwrap();

        let (config, path) = PawsConfig::discover(&package).unwrap();
        assert_eq!(config.toolchain_version("node"), Some("22"));
        assert_eq!(path.unwrap(), root.join(CONFIG_FILE));

        // A package-level config overrides the repo-wide one.
        std::fs::write(package.join(CONFIG_FILE), "[toolchains]\nnode = \"20\"\n").unwrap();
        let (config, _) = PawsConfig::discover(&package).unwrap();
        assert_eq!(config.toolchain_version("node"), Some("20"));

        std::fs::remove_dir_all(&root).ok();
    }
}
