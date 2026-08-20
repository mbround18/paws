//! Metadata for the GitHub Actions this repo ships (e.g. `actions/paws-up`),
//! embedded into the compiled binary at build time via `include_str!` —
//! `paws llms generate`/`paws mcp serve` typically run from an arbitrary
//! *consumer* repo's working directory, not paws's own checkout, so
//! `actions/*/action.yml` wouldn't exist on disk at runtime for a filesystem
//! walk to find.
//!
//! There is only one action today; it's listed explicitly rather than
//! discovered via a build-time glob — simple for a list of one, and
//! trivially extended (`(id, include_str!(...))`) if a second action ever
//! ships.

use anyhow::Context;

const EMBEDDED_ACTIONS: &[(&str, &str)] = &[(
    "paws-up",
    include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../actions/paws-up/action.yml"
    )),
)];

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ActionInput {
    pub name: String,
    pub description: String,
    pub required: bool,
    pub default: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ActionOutput {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ActionMetadata {
    /// Directory name under `actions/`, e.g. "paws-up" — what `uses:`
    /// actually references, distinct from `name` (the human-readable
    /// `name:` field inside `action.yml`, e.g. "paws up").
    pub id: String,
    pub name: String,
    /// Ready-to-paste `uses:` target, e.g.
    /// "mbround18/paws/actions/paws-up@main".
    pub usage: String,
    pub description: String,
    pub inputs: Vec<ActionInput>,
    pub outputs: Vec<ActionOutput>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawInput {
    #[serde(default)]
    description: String,
    #[serde(default)]
    required: bool,
    #[serde(default)]
    default: Option<serde_yaml::Value>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawOutput {
    #[serde(default)]
    description: String,
}

#[derive(Debug, Default, serde::Deserialize)]
struct RawActionManifest {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    inputs: std::collections::BTreeMap<String, RawInput>,
    #[serde(default)]
    outputs: std::collections::BTreeMap<String, RawOutput>,
}

/// A YAML scalar default (e.g. `default: "latest"`) renders as its plain
/// string; anything non-scalar (unusual for an action input, but not
/// impossible) falls back to its YAML text rather than being dropped.
fn default_to_string(value: serde_yaml::Value) -> String {
    match value {
        serde_yaml::Value::String(s) => s,
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        other => serde_yaml::to_string(&other)
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
}

/// Parses every embedded `action.yml` into structured metadata — inputs,
/// outputs, descriptions, and a ready-to-paste `uses:` string.
pub fn discover_actions() -> anyhow::Result<Vec<ActionMetadata>> {
    EMBEDDED_ACTIONS
        .iter()
        .map(|(id, yaml)| {
            let raw: RawActionManifest = serde_yaml::from_str(yaml)
                .with_context(|| format!("failed to parse embedded actions/{id}/action.yml"))?;

            let mut inputs: Vec<ActionInput> = raw
                .inputs
                .into_iter()
                .map(|(name, input)| ActionInput {
                    name,
                    description: input.description,
                    required: input.required,
                    default: input.default.map(default_to_string),
                })
                .collect();
            inputs.sort_by(|a, b| a.name.cmp(&b.name));

            let mut outputs: Vec<ActionOutput> = raw
                .outputs
                .into_iter()
                .map(|(name, output)| ActionOutput {
                    name,
                    description: output.description,
                })
                .collect();
            outputs.sort_by(|a, b| a.name.cmp(&b.name));

            Ok(ActionMetadata {
                id: id.to_string(),
                name: raw.name,
                usage: format!("mbround18/paws/actions/{id}@main"),
                description: raw.description,
                inputs,
                outputs,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_paws_up_with_its_known_inputs_and_output() {
        let actions = discover_actions().unwrap();
        let paws_up = actions
            .iter()
            .find(|a| a.id == "paws-up")
            .expect("paws-up action discovered");

        assert_eq!(paws_up.usage, "mbround18/paws/actions/paws-up@main");
        assert!(!paws_up.description.is_empty());

        let input_names: Vec<&str> = paws_up.inputs.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(
            input_names,
            vec!["github-token", "install-dagger", "version"]
        );

        let version = paws_up.inputs.iter().find(|i| i.name == "version").unwrap();
        assert!(!version.required);
        assert_eq!(version.default.as_deref(), Some("latest"));

        let output_names: Vec<&str> = paws_up.outputs.iter().map(|o| o.name.as_str()).collect();
        assert_eq!(output_names, vec!["version"]);
    }
}
