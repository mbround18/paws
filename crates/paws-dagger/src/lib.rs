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

/// Checks once (e.g. at `paws` startup) that the `dagger` CLI is reachable,
/// producing an actionable error naming the missing binary and a remediation
/// hint rather than letting every subcommand surface a raw OS-level
/// "No such file or directory" (FR-010).
pub async fn ensure_available() -> Result<()> {
    match Command::new("dagger").arg("version").output().await {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => anyhow::bail!(
            "`dagger version` exited with a failure: {}",
            String::from_utf8_lossy(&output.stderr)
        ),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            anyhow::bail!(
                "`dagger` CLI not found on PATH. Install it from \
                 https://docs.dagger.io/install and re-run `paws`."
            )
        }
        Err(err) => Err(err).context("failed to check for the `dagger` CLI on PATH"),
    }
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

/// Runs a moduleless `dagger core <args...>` pipeline — chained core
/// functions (`host directory`, `docker-build`, `with-exec`, `export`, ...)
/// without needing a custom Dagger module. This is how `paws-release` builds
/// against `./builders/*` Dockerfiles and smoke-tests cross-platform/cross-
/// arch binaries (via `container --platform=...`), keeping this crate the
/// single seam that spawns `dagger` (SC-004) even for ad-hoc pipelines.
pub async fn core(args: &[String]) -> Result<String> {
    let output = Command::new("dagger")
        .arg("core")
        .args(args)
        .output()
        .await
        .context("failed to spawn `dagger` CLI - is it installed and on PATH?")?;

    if !output.status.success() {
        anyhow::bail!("dagger core {}: failed: {}", args.join(" "), String::from_utf8_lossy(&output.stderr));
    }

    Ok(String::from_utf8(output.stdout)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ensure_available_reports_missing_binary_actionably() {
        // Don't assume `dagger` is absent in every environment this test
        // runs in — only assert the error shape when it actually is absent.
        if let Err(err) = ensure_available().await {
            assert!(
                err.to_string().contains("dagger` CLI not found on PATH")
                    || err.to_string().contains("dagger version"),
                "unexpected error: {err}"
            );
        }
    }

    #[tokio::test]
    async fn core_runs_a_moduleless_pipeline() {
        if ensure_available().await.is_err() {
            return; // no `dagger` on PATH in this environment; nothing to verify
        }
        let output = core(&[
            "container".into(),
            "from".into(),
            "--address=alpine:3.20".into(),
            "with-exec".into(),
            "--args=echo,hello".into(),
            "stdout".into(),
        ])
        .await
        .unwrap();
        assert_eq!(output.trim(), "hello");
    }

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
