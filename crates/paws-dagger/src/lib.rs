//! Thin wrapper around the `dagger` CLI. Every pipeline crate calls through
//! here rather than shelling out directly, so the day the Rust SDK is ready
//! to trust with real work, only this crate has to change.

use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// Attempt count and backoff (3s/6s/12s between attempts, ~21s worst case)
/// shared by every retry loop in this crate — matches
/// [`remote_image_exists_with_retry`]'s already-tuned values.
const RETRY_ATTEMPTS: u32 = 4;

async fn retry_backoff(attempt: u32) {
    let backoff_secs = 3u64 * 2u64.pow(attempt - 1);
    tokio::time::sleep(std::time::Duration::from_secs(backoff_secs)).await;
}

/// Whether `stderr` looks like a transient registry/network blip — the
/// same class of intermittent CloudFront/registry hiccup already handled
/// for [`remote_image_exists_with_retry`] — rather than a real build/test/
/// lint failure. Matching on these substrings keeps retrying narrow: a
/// genuine `cargo test` failure, `clippy` warning, or a truly missing
/// image never contains any of them, so a real failure still fails fast
/// instead of burning ~21s retrying something that was never going to
/// succeed.
fn is_transient_registry_error(stderr: &str) -> bool {
    const TRANSIENT_SIGNATURES: &[&str] = &[
        "failed to copy",
        "httpReadSeeker",
        "connection reset by peer",
        "i/o timeout",
        "tls handshake timeout",
        "unexpected eof",
        "context deadline exceeded",
        "dial tcp",
        "no such host",
        "too many requests",
        "toomanyrequests",
        "500 internal server error",
        "502 bad gateway",
        "503 service unavailable",
        "504 gateway timeout",
    ];
    let lower = stderr.to_lowercase();
    TRANSIENT_SIGNATURES.iter().any(|sig| lower.contains(sig))
}

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

/// Installs the `dagger` CLI itself via its official install script
/// (`https://dl.dagger.io/dagger/install.sh`) — the same script
/// `.github/workflows/ci.yaml`/`release.yaml` already use, ported here so
/// `paws init` (and `actions/paws-up`) don't need their own copy of it.
/// Not a `dagger`/`docker`/`cross` spawn itself (it shells to `sh`, running
/// the fetched installer script), so it sits outside ADR-0001's "route
/// container execution through Dagger" scope — this installs the tool that
/// scope is about, it doesn't execute a pipeline.
///
/// The script's own default install location (`./bin`, relative to
/// whatever the current directory happens to be — confirmed by reading the
/// script itself, not assumed) isn't useful for this; `BIN_DIR` is pinned
/// to `$HOME/.local/bin` instead, mirroring `actions/paws-up`'s own install
/// directory. In a GitHub Actions run (`$GITHUB_PATH` set), that directory
/// is also appended there so a later step's `dagger` calls resolve without
/// the caller having to touch `PATH` itself.
pub async fn install_cli() -> Result<std::path::PathBuf> {
    let home =
        std::env::var("HOME").context("HOME is not set - can't determine an install directory")?;
    let install_dir = std::path::PathBuf::from(home).join(".local").join("bin");
    tokio::fs::create_dir_all(&install_dir)
        .await
        .context("failed to create the dagger install directory")?;

    let output = Command::new("sh")
        .arg("-c")
        .arg("curl -fsSL https://dl.dagger.io/dagger/install.sh | sh")
        .env("BIN_DIR", &install_dir)
        .output()
        .await
        .context("failed to run the dagger install script")?;

    if !output.status.success() {
        anyhow::bail!(
            "dagger install script failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    if let Ok(github_path) = std::env::var("GITHUB_PATH") {
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .open(&github_path)
            .await
            .context("failed to open $GITHUB_PATH for appending")?;
        file.write_all(format!("{}\n", install_dir.display()).as_bytes())
            .await
            .context("failed to append the dagger install directory to $GITHUB_PATH")?;
    }

    Ok(install_dir)
}

/// Whether `image` (e.g. `ghcr.io/mbround18/paws-builders/linux-gnu:v0.0.1`)
/// can actually be pulled — used to decide between starting a `dagger core`
/// pipeline from a prebuilt registry image (`container from --address=...`)
/// versus building `./builders/<name>/Dockerfile` locally from scratch.
/// Goes through `dagger core container from ... platform` (the sanctioned
/// container-execution seam per ADR-0001) rather than a `docker manifest
/// inspect`/registry-API probe, so this never becomes a second place that
/// talks to a registry directly. `platform` (not `id`, which isn't a valid
/// terminal call on `container from` — verified directly against a real
/// `dagger` CLI, not assumed) is just the cheapest real scalar that forces
/// Dagger to actually resolve the image.
pub async fn remote_image_exists(image: &str) -> bool {
    // Single-shot, not `core` (which already retries transient errors
    // itself): `remote_image_exists_with_retry` below is the retrying
    // entry point for this check, with its own backoff tuned for
    // existence-checking specifically — going through `core`'s retry too
    // would nest two 4-attempt loops for the same transient failure.
    core_once(&[
        "container".into(),
        "from".into(),
        format!("--address={image}"),
        "platform".into(),
    ])
    .await
    .is_ok()
}

/// [`remote_image_exists`], retried with backoff before giving up — a
/// separate build-builders job pushed `image` moments earlier in the same
/// workflow run, and both registry read-after-write propagation and
/// transient pull errors have been observed to make a real, freshly-pushed
/// image briefly report as missing here. 4 attempts, 3s/6s/12s backoff
/// between them (~21s worst case) is enough headroom for that without
/// masking a genuinely absent image for long.
pub async fn remote_image_exists_with_retry(image: &str) -> bool {
    for attempt in 1..=RETRY_ATTEMPTS {
        if remote_image_exists(image).await {
            return true;
        }
        if attempt < RETRY_ATTEMPTS {
            retry_backoff(attempt).await;
        }
    }
    false
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

async fn core_once(args: &[String]) -> Result<String> {
    let output = Command::new("dagger")
        .arg("core")
        .args(args)
        .output()
        .await
        .context("failed to spawn `dagger` CLI - is it installed and on PATH?")?;

    if !output.status.success() {
        anyhow::bail!(
            "dagger core {}: failed: {}",
            args.join(" "),
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
///
/// Every pipeline built here starts with `container from --address=...`
/// (or pulls further images mid-chain, e.g. a multi-stage `docker-build`),
/// so a transient registry/CloudFront blip anywhere in the chain fails the
/// whole pipeline — a real, observed failure
/// (mbround18/paws#5's CI run: `rust:1-bookworm` pull reset mid-transfer)
/// that had nothing to do with the code under test. Retried the same way
/// as [`remote_image_exists_with_retry`] ([`is_transient_registry_error`]
/// gates it to actual network/registry signatures, so a real build/test/
/// lint failure still fails on the first attempt instead of burning ~21s).
pub async fn core(args: &[String]) -> Result<String> {
    for attempt in 1..=RETRY_ATTEMPTS {
        match core_once(args).await {
            Ok(stdout) => return Ok(stdout),
            Err(err)
                if attempt < RETRY_ATTEMPTS && is_transient_registry_error(&err.to_string()) =>
            {
                eprintln!(
                    "dagger core: transient registry error (attempt {attempt}/{RETRY_ATTEMPTS}), retrying: {err}"
                );
                retry_backoff(attempt).await;
            }
            Err(err) => return Err(err),
        }
    }
    unreachable!("loop always returns by the final attempt")
}

/// Same pipeline as [`core`], but streams `dagger`'s own live progress
/// output straight to this process's stdout/stderr as it runs, instead of
/// buffering everything and only printing it once the whole pipeline
/// finishes. Capturing it via `.output()` (as [`core`] does) throws that
/// away and leaves a caller sitting with no visible progress on a build
/// that can take minutes, only ever seeing the small handful of
/// `println!`s a `paws ci` caller wraps around it.
///
/// Uses `--progress=plain`, not `dagger core`'s default renderer —
/// verified directly that the default renderer redraws in place via
/// cursor-repositioning escape codes, which only makes sense on a real
/// TTY: piped to a file (the same situation a GitHub Actions log is in),
/// it writes nothing at all until the pipeline finishes, then dumps
/// everything at once — exactly the "nothing for minutes, then everything"
/// behavior this function exists to fix. `--progress=plain` is append-only
/// and was confirmed (via a deliberately slow, non-cacheable step) to
/// write lines incrementally as they happen, under a redirected/piped
/// stdout, not just on a real terminal.
///
/// This is what `paws ci` uses by default now; [`core`] (captured, silent
/// until done) remains for `--silent` and for callers that need the
/// output text itself, not just pass/fail.
///
/// stderr is piped (not inherited) so a transient registry blip can be
/// detected and retried the same way [`core`] does, but every chunk read
/// is written straight back out to this process's real stderr as it
/// arrives — the same live, incremental behavior as full inheritance,
/// just tapped for a copy. stdout stays directly inherited: `dagger`'s own
/// `--progress=plain` output and error text land on stderr, and nothing
/// here needs to inspect stdout.
async fn core_streaming_once(args: &[String]) -> Result<(bool, String)> {
    let mut child = Command::new("dagger")
        .arg("core")
        .arg("--progress=plain")
        .args(args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn `dagger` CLI - is it installed and on PATH?")?;

    let mut child_stderr = child
        .stderr
        .take()
        .expect("stderr was configured as piped above");
    let mut captured = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = child_stderr.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        use std::io::Write;
        std::io::stderr().write_all(&chunk[..n])?;
        captured.extend_from_slice(&chunk[..n]);
    }

    let status = child.wait().await?;
    Ok((
        status.success(),
        String::from_utf8_lossy(&captured).into_owned(),
    ))
}

pub async fn core_streaming(args: &[String]) -> Result<()> {
    for attempt in 1..=RETRY_ATTEMPTS {
        let (success, captured_stderr) = core_streaming_once(args).await?;
        if success {
            return Ok(());
        }
        if attempt < RETRY_ATTEMPTS && is_transient_registry_error(&captured_stderr) {
            eprintln!(
                "dagger core: transient registry error (attempt {attempt}/{RETRY_ATTEMPTS}), retrying..."
            );
            retry_backoff(attempt).await;
            continue;
        }
        anyhow::bail!("dagger core {}: failed (see output above)", args.join(" "));
    }
    unreachable!("loop always returns or bails by the final attempt")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_real_observed_registry_errors_as_transient() {
        // The exact wording seen on a real CI run (mbround18/paws#5,
        // pulling rust:1-bookworm mid-transfer).
        assert!(is_transient_registry_error(
            r#"pull image "docker.io/library/rust:1-bookworm@sha256:...": failed to copy: httpReadSeeker: failed open: failed to do request: Get "https://production.cloudfront.docker.com/...": read tcp 172.17.0.2:44604->3.170.185.54:443: read: connection reset by peer"#
        ));
        assert!(is_transient_registry_error("dial tcp: i/o timeout"));
        assert!(is_transient_registry_error("429 Too Many Requests"));
        assert!(is_transient_registry_error("received 502 Bad Gateway"));
        assert!(is_transient_registry_error("TLS handshake timeout"));
    }

    #[test]
    fn does_not_classify_a_real_build_failure_as_transient() {
        assert!(!is_transient_registry_error(
            "error[E0425]: cannot find function `foo` in this scope"
        ));
        assert!(!is_transient_registry_error(
            "test result: FAILED. 1 passed; 1 failed"
        ));
        assert!(!is_transient_registry_error(
            r#"pull image "docker.io/library/definitely-not-a-real-image:latest": not found"#
        ));
    }

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

    #[tokio::test]
    async fn core_streaming_succeeds_on_a_real_pipeline_and_fails_on_a_bad_one() {
        if ensure_available().await.is_err() {
            return; // no `dagger` on PATH in this environment; nothing to verify
        }
        core_streaming(&[
            "container".into(),
            "from".into(),
            "--address=alpine:3.20".into(),
            "with-exec".into(),
            "--args=echo,hello".into(),
            "stdout".into(),
        ])
        .await
        .unwrap();

        let err = core_streaming(&[
            "container".into(),
            "from".into(),
            "--address=alpine:3.20".into(),
            "with-exec".into(),
            "--args=false".into(),
            "stdout".into(),
        ])
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("failed"),
            "unexpected error: {err}"
        );
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
