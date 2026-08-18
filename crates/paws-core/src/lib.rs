//! Shared contract types for paws pipelines: defaults, workflow definitions,
//! and the config shapes every language-specific crate builds on.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineDefaults {
    pub toolchain: Option<String>,
    pub registry: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_roundtrip_through_json() {
        let defaults = PipelineDefaults {
            toolchain: Some("stable".into()),
            registry: None,
        };
        let json = serde_json::to_string(&defaults).unwrap();
        let parsed: PipelineDefaults = serde_json::from_str(&json).unwrap();
        assert_eq!(defaults, parsed);
    }
}
