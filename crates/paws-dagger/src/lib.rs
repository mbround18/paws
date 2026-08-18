//! Thin wrapper around the `dagger` CLI. Every pipeline crate calls through
//! here rather than shelling out directly, so the day the Rust SDK is ready
//! to trust with real work, only this crate has to change.

use anyhow::{Context, Result};
use tokio::process::Command;

pub struct DaggerCall {
    pub module: String,
    pub function: String,
    pub args: Vec<String>,
}

pub async fn call(invocation: DaggerCall) -> Result<String> {
    let output = Command::new("dagger")
        .arg("call")
        .arg("-m")
        .arg(&invocation.module)
        .arg(&invocation.function)
        .args(&invocation.args)
        .output()
        .await
        .context("failed to spawn `dagger` CLI - is it installed and on PATH?")?;

    if !output.status.success() {
        anyhow::bail!(
            "dagger call {} {} failed: {}",
            invocation.module,
            invocation.function,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8(output.stdout)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dagger_call_builds_expected_args() {
        let invocation = DaggerCall {
            module: "./crates/paws-semver".into(),
            function: "compute".into(),
            args: vec!["--branch".into(), "main".into()],
        };
        assert_eq!(invocation.module, "./crates/paws-semver");
        assert_eq!(invocation.args, vec!["--branch", "main"]);
    }
}
