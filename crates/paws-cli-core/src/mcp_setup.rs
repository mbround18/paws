//! `paws mcp setup`: write/merge an MCP client config so `paws mcp serve` is
//! discoverable. Pure file I/O — no dependency on `paws-mcp` itself, so this
//! lives in `paws-cli`'s own lib rather than the server crate.

use anyhow::Context;

use crate::McpSetupArgs;

const SERVER_NAME: &str = "paws";

fn server_entry() -> serde_json::Value {
    serde_json::json!({
        "command": "paws",
        "args": ["mcp", "serve"],
    })
}

/// Reads `path` as JSON if it exists (empty object otherwise), inserts/
/// overwrites `mcpServers.paws`, and writes it back — preserving any other
/// server entries already configured.
fn merge_mcp_config(path: &std::path::Path) -> anyhow::Result<()> {
    let mut config: serde_json::Value = if path.exists() {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        serde_json::from_str(&raw)
            .with_context(|| format!("{} is not valid JSON", path.display()))?
    } else {
        serde_json::json!({})
    };

    if !config.is_object() {
        anyhow::bail!(
            "{} does not contain a JSON object at its root",
            path.display()
        );
    }
    let root = config.as_object_mut().expect("checked above");
    let servers = root
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}));
    if !servers.is_object() {
        anyhow::bail!(
            "{}'s \"mcpServers\" key is not a JSON object",
            path.display()
        );
    }
    servers
        .as_object_mut()
        .expect("checked above")
        .insert(SERVER_NAME.to_string(), server_entry());

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let pretty = serde_json::to_string_pretty(&config)?;
    std::fs::write(path, pretty + "\n")
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

/// Platform-specific path to Claude Desktop's global MCP config.
fn claude_desktop_config_path() -> anyhow::Result<std::path::PathBuf> {
    if cfg!(target_os = "macos") {
        let home = std::env::var("HOME").context("$HOME is not set")?;
        Ok(std::path::PathBuf::from(home)
            .join("Library/Application Support/Claude/claude_desktop_config.json"))
    } else if cfg!(target_os = "windows") {
        let appdata = std::env::var("APPDATA").context("$APPDATA is not set")?;
        Ok(std::path::PathBuf::from(appdata).join("Claude/claude_desktop_config.json"))
    } else {
        let config_home = std::env::var("XDG_CONFIG_HOME").ok().unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_default();
            format!("{home}/.config")
        });
        Ok(std::path::PathBuf::from(config_home).join("Claude/claude_desktop_config.json"))
    }
}

// `async` with nothing to await, deliberately: every `run_*` entry point
// shares one signature so `execute`'s dispatch and `paws-mcp`'s tool
// handlers can call them uniformly. Dropping it here would make this the
// one command both callers have to special-case.
#[allow(clippy::unused_async)]
pub async fn run_mcp_setup(args: McpSetupArgs) -> anyhow::Result<()> {
    let client = args.client.as_deref().unwrap_or("claude-code");
    match client {
        "claude-code" => {
            let path = std::path::Path::new(".mcp.json");
            merge_mcp_config(path)?;
            println!(
                "mcp setup: wrote {} (\"{SERVER_NAME}\" -> `paws mcp serve`)",
                path.display()
            );
        }
        "claude-desktop" => {
            let path = claude_desktop_config_path()?;
            merge_mcp_config(&path)?;
            println!(
                "mcp setup: wrote {} (\"{SERVER_NAME}\" -> `paws mcp serve`)",
                path.display()
            );
        }
        other => anyhow::bail!(
            "unsupported --client '{other}'; expected 'claude-code' or 'claude-desktop'"
        ),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each test gets its own scratch subdirectory (named after the test)
    /// under a shared temp root, so parallel `cargo test` runs can't collide
    /// on the same `.mcp.json` path.
    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("paws-mcp-setup-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn merge_preserves_existing_servers() {
        let dir = scratch_dir("merge-preserves");
        let path = dir.join(".mcp.json");
        std::fs::write(
            &path,
            r#"{"mcpServers": {"other": {"command": "other-tool"}}}"#,
        )
        .unwrap();

        merge_mcp_config(&path).unwrap();

        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            written["mcpServers"]["other"]["command"],
            serde_json::json!("other-tool")
        );
        assert_eq!(
            written["mcpServers"]["paws"]["args"],
            serde_json::json!(["mcp", "serve"])
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn merge_creates_a_fresh_config_when_none_exists_yet() {
        let dir = scratch_dir("merge-fresh");
        let path = dir.join("nested").join(".mcp.json");
        assert!(!path.exists());

        merge_mcp_config(&path).unwrap();

        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            written["mcpServers"]["paws"]["command"],
            serde_json::json!("paws")
        );
        assert_eq!(
            written["mcpServers"].as_object().unwrap().len(),
            1,
            "a fresh config should contain nothing but the paws entry"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn merge_rejects_invalid_json() {
        let dir = scratch_dir("merge-invalid-json");
        let path = dir.join(".mcp.json");
        std::fs::write(&path, "not json").unwrap();

        let err = merge_mcp_config(&path).unwrap_err();
        assert!(err.to_string().contains("is not valid JSON"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn merge_rejects_a_non_object_root() {
        let dir = scratch_dir("merge-non-object-root");
        let path = dir.join(".mcp.json");
        std::fs::write(&path, "[1, 2, 3]").unwrap();

        let err = merge_mcp_config(&path).unwrap_err();
        assert!(err.to_string().contains("does not contain a JSON object"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn merge_rejects_a_non_object_mcp_servers_key() {
        let dir = scratch_dir("merge-non-object-servers");
        let path = dir.join(".mcp.json");
        std::fs::write(&path, r#"{"mcpServers": "oops"}"#).unwrap();

        let err = merge_mcp_config(&path).unwrap_err();
        assert!(
            err.to_string()
                .contains("\"mcpServers\" key is not a JSON object")
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn run_mcp_setup_rejects_an_unsupported_client() {
        let err = run_mcp_setup(McpSetupArgs {
            client: Some("cursor".to_string()),
        })
        .await
        .unwrap_err();
        assert!(err.to_string().contains("unsupported --client 'cursor'"));
    }

    #[test]
    fn claude_desktop_config_path_lands_under_a_claude_directory() {
        // Only meaningful on the non-macOS/non-Windows (XDG) branch, which
        // is what CI (ubuntu-latest) actually exercises; the macOS/Windows
        // branches are `cfg!`-gated on the host OS and can't be exercised
        // from a Linux test run.
        if cfg!(any(target_os = "macos", target_os = "windows")) {
            return;
        }
        let path = claude_desktop_config_path().unwrap();
        assert!(path.ends_with("Claude/claude_desktop_config.json"));
    }
}
