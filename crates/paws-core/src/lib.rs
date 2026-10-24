//! Shared contract types for paws pipelines: defaults, workflow definitions,
//! and the config shapes every language-specific crate builds on.

pub mod builders;
pub mod config;
pub mod container;
pub mod fsutil;
pub mod pipeline;
#[cfg(any(test, feature = "testing"))]
pub mod test_support;
pub mod toolchain;
pub mod version;

pub use builders::write_builder_dockerfile;
pub use config::PawsConfig;
pub use container::{Base, CacheMount, ContainerOptions, Export};
pub use fsutil::{find_files, find_files_with_extension};
pub use pipeline::{GENERATED_DOCKERFILE, Pipeline, builder_build_args, csv_join};
pub use toolchain::{TOOLCHAINS, Toolchain, ToolchainInfo};
pub use version::{ResolvedVersion, VersionOrigin, VersionSource};

use serde::{Deserialize, Serialize};

const fn default_mod_upload_validate_before_upload() -> bool {
    true
}

#[allow(clippy::unnecessary_wraps)]
fn default_mod_upload_environment_preference() -> Option<String> {
    Some("github_environment".to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineDefaults {
    pub toolchain: Option<String>,
    pub registry: Option<String>,
    /// Default `paws changelog` output path when `--output` is omitted.
    /// `#[serde(default)]` is required (not implied by `Option<T>` alone)
    /// so an already-serialized `PipelineDefaults` payload missing this key
    /// still deserializes after this field was added.
    #[serde(default)]
    pub changelog_path: Option<String>,
    /// Safe-by-default mod publish toggle: repository callers must opt in.
    #[serde(default)]
    pub mod_upload_enabled: bool,
    /// Validation is a hard gate before any registry publication.
    #[serde(default = "default_mod_upload_validate_before_upload")]
    pub mod_upload_validate_before_upload: bool,
    /// Validation-only and dry-run flows return structured status without
    /// publishing to the remote registry.
    #[serde(default)]
    pub mod_upload_dry_run: bool,
    /// Explicit provider name when mod publishing is enabled.
    #[serde(default)]
    pub mod_upload_provider: Option<String>,
    /// Prefer GitHub Environment-scoped credentials when present.
    #[serde(default = "default_mod_upload_environment_preference")]
    pub mod_upload_environment_preference: Option<String>,
}

impl Default for PipelineDefaults {
    fn default() -> Self {
        Self {
            toolchain: None,
            registry: None,
            changelog_path: None,
            mod_upload_enabled: false,
            mod_upload_validate_before_upload: true,
            mod_upload_dry_run: false,
            mod_upload_provider: None,
            mod_upload_environment_preference: Some("github_environment".to_string()),
        }
    }
}

/// `PipelineDefaults::changelog_path`'s fallback when unset.
pub const DEFAULT_CHANGELOG_PATH: &str = "CHANGELOG.md";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_roundtrip_through_json() {
        let defaults = PipelineDefaults {
            toolchain: Some("stable".into()),
            registry: None,
            changelog_path: Some("CHANGELOG.md".into()),
            mod_upload_enabled: true,
            mod_upload_validate_before_upload: true,
            mod_upload_dry_run: false,
            mod_upload_provider: Some("hexium".into()),
            mod_upload_environment_preference: Some("github_environment".into()),
        };
        let json = serde_json::to_string(&defaults).unwrap();
        let parsed: PipelineDefaults = serde_json::from_str(&json).unwrap();
        assert_eq!(defaults, parsed);
    }

    #[test]
    fn changelog_path_defaults_to_none_when_missing_from_serialized_json() {
        let json = r#"{"toolchain":"stable","registry":null}"#;
        let parsed: PipelineDefaults = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.changelog_path, None);
    }

    #[test]
    fn mod_upload_defaults_are_safe_and_opt_in() {
        let json = r#"{"toolchain":"stable","registry":null}"#;
        let parsed: PipelineDefaults = serde_json::from_str(json).unwrap();
        assert!(!parsed.mod_upload_enabled);
        assert!(parsed.mod_upload_validate_before_upload);
        assert!(!parsed.mod_upload_dry_run);
        assert_eq!(parsed.mod_upload_provider, None);
        assert_eq!(
            parsed.mod_upload_environment_preference,
            Some("github_environment".to_string())
        );
    }
}
