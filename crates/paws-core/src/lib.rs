//! Shared contract types for paws pipelines: defaults, workflow definitions,
//! and the config shapes every language-specific crate builds on.

pub mod builders;
pub mod fsutil;
pub mod pipeline;
#[cfg(any(test, feature = "testing"))]
pub mod test_support;
pub mod toolchain;

pub use builders::write_builder_dockerfile;
pub use fsutil::{find_files, find_files_with_extension};
pub use pipeline::{Pipeline, builder_build_args};
pub use toolchain::{TOOLCHAINS, Toolchain, ToolchainInfo};

use serde::{Deserialize, Serialize};

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
}
