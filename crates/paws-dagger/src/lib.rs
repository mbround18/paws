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

/// specs/005-close-remaining-cli: which remote build-cache mechanism (if
/// any) [`core`]/[`core_streaming`] wrap their `dagger` invocation with, so
/// layers survive across separate, ephemeral CI runs instead of rebuilding
/// from zero every time.
///
/// Selection is automatic (no CLI flag anywhere) with a fixed precedence:
/// `DaggerCloud` wins when both signatures are present. This does **not**
/// make `DaggerCloud` the "primary" provider in implementation priority —
/// it needs a paid Dagger Cloud account, while `GitHubActionsCache` needs
/// no external account at all and is what most GitHub Actions-only
/// consumers will actually depend on (research.md R7's explicit note).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheBackend {
    /// `$DAGGER_CLOUD_TOKEN` is set — Dagger's own native remote cache.
    /// Needs no code here beyond detection: `Command::new("dagger")`
    /// inherits the full parent environment by default (no env-clearing
    /// calls anywhere in this crate), so the token already reaches the
    /// subprocess unmodified (research.md R6).
    DaggerCloud,
    /// Running inside a real GitHub Actions job with the legacy Cache
    /// Service v1 REST API available (`$ACTIONS_CACHE_URL` +
    /// `$ACTIONS_RUNTIME_TOKEN`). Wraps the local Dagger engine's
    /// persistent state (a named Docker volume mounted at
    /// `/var/lib/dagger` inside the `dagger-engine-v<version>` container —
    /// confirmed for real via `docker inspect` against a live engine, not
    /// assumed) with a restore-before/save-after pair around the pipeline
    /// call.
    GitHubActionsCache { base_url: String, token: String },
    /// Neither signature present — today's behavior, unchanged (FR-007).
    None,
}

impl CacheBackend {
    /// Detects which backend to use from the current process environment.
    /// `DAGGER_CLOUD_TOKEN` wins when both signatures are present (FR-005).
    ///
    /// Deliberately checks `$ACTIONS_CACHE_URL` (the legacy Cache Service
    /// v1 REST API, JSON-based) rather than `$ACTIONS_RESULTS_URL` (the
    /// newer Twirp/protobuf-based results service) — the latter is a
    /// materially different wire protocol this crate doesn't implement yet
    /// (tracked in docs/ROADMAP.md); a runner that only sets
    /// `$ACTIONS_RESULTS_URL` falls through to `None` rather than a
    /// nonfunctional half-implementation.
    pub fn detect() -> Self {
        if std::env::var("DAGGER_CLOUD_TOKEN").is_ok() {
            return CacheBackend::DaggerCloud;
        }
        if let (Ok(base_url), Ok(token)) = (
            std::env::var("ACTIONS_CACHE_URL"),
            std::env::var("ACTIONS_RUNTIME_TOKEN"),
        ) {
            return CacheBackend::GitHubActionsCache { base_url, token };
        }
        CacheBackend::None
    }

    /// FR-006: independently verifiable without inferring from build speed
    /// — exactly one line, naming the selected backend (or `none`).
    pub fn log_line(&self) -> String {
        match self {
            CacheBackend::DaggerCloud => "cache: using dagger-cloud".to_string(),
            CacheBackend::GitHubActionsCache { .. } => "cache: using github-actions".to_string(),
            CacheBackend::None => "cache: no backend detected (full rebuild)".to_string(),
        }
    }
}

/// Name prefix every `dagger`-CLI-managed engine container uses
/// (`dagger-engine-v<version>`) — confirmed for real via `docker ps`
/// against a live `dagger` v0.21.8 install, not assumed from
/// documentation.
const ENGINE_CONTAINER_NAME_PREFIX: &str = "dagger-engine-";

/// Path inside the engine container where its persistent state (BuildKit
/// cache, layer store) lives — confirmed for real via `docker inspect`
/// against a live engine container's `Mounts`, not assumed.
const ENGINE_STATE_PATH: &str = "/var/lib/dagger";

/// The GitHub Actions Cache Service v1 REST API's fixed API version query
/// param — bumping the Dagger CLI's own version changes what a cached
/// engine-state archive is compatible with, so it's folded into the cache
/// key rather than the API version.
const CACHE_API_VERSION: &str = "1.0";

async fn docker_output(args: &[&str]) -> Result<String> {
    let output = Command::new("docker")
        .args(args)
        .output()
        .await
        .with_context(|| format!("failed to spawn `docker {}`", args.join(" ")))?;
    if !output.status.success() {
        anyhow::bail!(
            "docker {}: failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Finds the currently-known `dagger-engine-v*` container's name, if the
/// engine has ever been started on this host. `None` means the engine
/// hasn't been created yet (e.g. the very first `dagger` invocation on a
/// fresh runner) — the caller is responsible for forcing engine creation
/// first if it needs the volume to exist (see [`ensure_engine_started`]).
async fn find_engine_container() -> Result<Option<String>> {
    let names = docker_output(&[
        "ps",
        "-a",
        "--filter",
        &format!("name={ENGINE_CONTAINER_NAME_PREFIX}"),
        "--format",
        "{{.Names}}",
    ])
    .await?;
    Ok(names.lines().next().map(str::to_string))
}

/// Forces the Dagger engine container (and its state volume) into
/// existence via the cheapest real pipeline call that still touches the
/// engine — `dagger version` alone does not start it (confirmed for real:
/// only an actual `core`/`call` invocation triggers the "starting engine /
/// create container" sequence).
async fn ensure_engine_started() -> Result<String> {
    if let Some(name) = find_engine_container().await? {
        return Ok(name);
    }
    core_once(&[
        "container".into(),
        "from".into(),
        "--address=alpine:3.20".into(),
        "platform".into(),
    ])
    .await
    .context("failed to start the Dagger engine for cache restore")?;
    find_engine_container()
        .await?
        .context("Dagger engine still not found after a warm-up call")
}

/// The named Docker volume backing `container`'s [`ENGINE_STATE_PATH`]
/// mount.
async fn find_engine_volume(container: &str) -> Result<String> {
    let volume = docker_output(&[
        "inspect",
        container,
        "--format",
        &format!(
            "{{{{ range .Mounts }}}}{{{{ if eq .Destination \"{ENGINE_STATE_PATH}\" }}}}{{{{ .Name }}}}{{{{ end }}}}{{{{ end }}}}"
        ),
    ])
    .await?;
    anyhow::ensure!(
        !volume.is_empty(),
        "no volume mounted at {ENGINE_STATE_PATH} on {container} — Dagger's own storage layout may have changed"
    );
    Ok(volume)
}

/// A stable cache key for the engine-state archive — scoped to the
/// `dagger` CLI's own version (a cache built by one engine version isn't
/// guaranteed compatible with another) plus a fixed prefix so it's easy to
/// recognize/invalidate deliberately.
async fn engine_cache_key() -> Result<String> {
    let output = Command::new("dagger")
        .arg("version")
        .output()
        .await
        .context("failed to spawn `dagger version` for the cache key")?;
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(format!("paws-dagger-engine-state-{version}"))
}

/// Minimal client for the GitHub Actions Cache Service **v1 REST API**
/// (`$ACTIONS_CACHE_URL`) — the same API `actions/cache@vN` itself calls.
/// Deliberately not the newer Twirp/protobuf results service
/// (`$ACTIONS_RESULTS_URL`, see [`CacheBackend::detect`]'s doc comment).
struct ActionsCacheClient {
    base_url: String,
    token: String,
    client: reqwest::Client,
}

impl ActionsCacheClient {
    fn new(base_url: String, token: String) -> Self {
        let base_url = base_url.trim_end_matches('/').to_string();
        Self {
            base_url,
            token,
            client: reqwest::Client::new(),
        }
    }

    fn auth_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder.bearer_auth(&self.token).header(
            "Accept",
            format!("application/json;api-version={CACHE_API_VERSION}"),
        )
    }

    /// `GET _apis/artifactcache/cache?keys=<key>&version=<key>` — `None` on
    /// a cache miss (204/404), else the signed archive download URL.
    async fn find_entry(&self, key: &str) -> Result<Option<String>> {
        let url = format!(
            "{}/_apis/artifactcache/cache?keys={key}&version={key}",
            self.base_url
        );
        let response = self
            .auth_headers(self.client.get(&url))
            .send()
            .await
            .context("failed to query the Actions cache for an existing entry")?;

        if response.status() == reqwest::StatusCode::NO_CONTENT
            || response.status() == reqwest::StatusCode::NOT_FOUND
        {
            return Ok(None);
        }
        if !response.status().is_success() {
            anyhow::bail!(
                "unexpected status querying the Actions cache: {}",
                response.status()
            );
        }
        let body: serde_json::Value = response
            .json()
            .await
            .context("failed to parse the Actions cache lookup response")?;
        Ok(body
            .get("archiveLocation")
            .and_then(|v| v.as_str())
            .map(String::from))
    }

    async fn download(&self, archive_location: &str, dest: &std::path::Path) -> Result<()> {
        let bytes = self
            .client
            .get(archive_location)
            .send()
            .await
            .context("failed to download the cached engine-state archive")?
            .bytes()
            .await
            .context("failed to read the cached engine-state archive body")?;
        tokio::fs::write(dest, &bytes)
            .await
            .context("failed to write the downloaded cache archive to disk")?;
        Ok(())
    }

    /// `POST _apis/artifactcache/caches` — reserves a cache entry, returns
    /// its id. The whole archive is uploaded as one chunk (a valid, if
    /// non-optimal, use of the API — chunked multi-part upload for very
    /// large archives is a documented follow-up, not a correctness gap for
    /// this first cut).
    async fn reserve(&self, key: &str, size: u64) -> Result<u64> {
        let url = format!("{}/_apis/artifactcache/caches", self.base_url);
        let body = serde_json::json!({ "key": key, "version": key, "cacheSize": size });
        let response = self
            .auth_headers(self.client.post(&url))
            .json(&body)
            .send()
            .await
            .context("failed to reserve an Actions cache entry")?;
        if !response.status().is_success() {
            anyhow::bail!(
                "failed to reserve an Actions cache entry: {}",
                response.status()
            );
        }
        let parsed: serde_json::Value = response
            .json()
            .await
            .context("failed to parse the Actions cache reservation response")?;
        parsed
            .get("cacheId")
            .and_then(|v| v.as_u64())
            .context("Actions cache reservation response missing cacheId")
    }

    async fn upload(&self, cache_id: u64, data: &[u8]) -> Result<()> {
        let url = format!("{}/_apis/artifactcache/caches/{cache_id}", self.base_url);
        let response = self
            .auth_headers(self.client.patch(&url))
            .header("Content-Type", "application/octet-stream")
            .header(
                "Content-Range",
                format!("bytes 0-{}/*", data.len().saturating_sub(1)),
            )
            .body(data.to_vec())
            .send()
            .await
            .context("failed to upload the engine-state archive to the Actions cache")?;
        if !response.status().is_success() {
            anyhow::bail!(
                "failed to upload to the Actions cache: {}",
                response.status()
            );
        }
        Ok(())
    }

    async fn commit(&self, cache_id: u64, size: u64) -> Result<()> {
        let url = format!("{}/_apis/artifactcache/caches/{cache_id}", self.base_url);
        let response = self
            .auth_headers(self.client.post(&url))
            .json(&serde_json::json!({ "size": size }))
            .send()
            .await
            .context("failed to commit the Actions cache entry")?;
        if !response.status().is_success() {
            anyhow::bail!(
                "failed to commit the Actions cache entry: {}",
                response.status()
            );
        }
        Ok(())
    }
}

/// Restores the Dagger engine's persistent state from the Actions cache,
/// if a cached entry exists, **before** the caller's real pipeline runs.
/// Safe no-op on a cache miss. Stops the engine container while restoring
/// (BuildKit needs exclusive access to its own on-disk state) — the next
/// real `dagger core`/`dagger call` invocation auto-restarts it, the same
/// `docker start dagger-engine-v...` sequence `dagger` already performs on
/// every invocation when the container exists but isn't running.
async fn restore_github_actions_cache(client: &ActionsCacheClient) -> Result<()> {
    let key = engine_cache_key().await?;
    let Some(archive_location) = client.find_entry(&key).await? else {
        eprintln!("cache: no existing github-actions cache entry for {key}, starting cold");
        return Ok(());
    };

    let container = ensure_engine_started().await?;
    let volume = find_engine_volume(&container).await?;

    let archive_path = std::env::temp_dir().join("paws-dagger-cache-restore.tar.gz");
    client.download(&archive_location, &archive_path).await?;

    docker_output(&["stop", &container]).await?;
    let extract = docker_output(&[
        "run",
        "--rm",
        "-v",
        &format!("{volume}:/data"),
        "-v",
        &format!("{}:/backup.tar.gz", archive_path.display()),
        "alpine:3.20",
        "sh",
        "-c",
        "tar xzf /backup.tar.gz -C /data",
    ])
    .await;
    let _ = tokio::fs::remove_file(&archive_path).await;
    extract.context("failed to extract the cached engine state into the Dagger volume")?;

    eprintln!("cache: restored github-actions cache entry {key}");
    Ok(())
}

/// Saves the Dagger engine's persistent state to the Actions cache
/// **after** the caller's real pipeline has run. Stops the engine
/// container for a consistent snapshot, tars the volume via a throwaway
/// helper container, uploads it, then restarts the engine so it's left
/// running for whatever runs next.
async fn save_github_actions_cache(client: &ActionsCacheClient) -> Result<()> {
    let Some(container) = find_engine_container().await? else {
        // The pipeline that just ran never actually started the engine
        // (e.g. it failed before any real dagger call) — nothing to save.
        return Ok(());
    };
    let volume = find_engine_volume(&container).await?;
    let key = engine_cache_key().await?;

    let archive_path = std::env::temp_dir().join("paws-dagger-cache-save.tar.gz");
    docker_output(&["stop", &container]).await?;
    let tar_result = docker_output(&[
        "run",
        "--rm",
        "-v",
        &format!("{volume}:/data"),
        "-v",
        &format!("{}:/backup.tar.gz", archive_path.display()),
        "alpine:3.20",
        "sh",
        "-c",
        "tar czf /backup.tar.gz -C /data .",
    ])
    .await;
    // Restart the engine regardless of whether the tar succeeded — leaving
    // it stopped would break the *next* pipeline call for a reason
    // unrelated to that call at all.
    let _ = docker_output(&["start", &container]).await;
    tar_result.context("failed to archive the Dagger volume for the Actions cache")?;

    let data = tokio::fs::read(&archive_path)
        .await
        .context("failed to read the archived engine state back from disk")?;
    let _ = tokio::fs::remove_file(&archive_path).await;

    let cache_id = client.reserve(&key, data.len() as u64).await?;
    client.upload(cache_id, &data).await?;
    client.commit(cache_id, data.len() as u64).await?;
    eprintln!(
        "cache: saved github-actions cache entry {key} ({} bytes)",
        data.len()
    );
    Ok(())
}

/// Detects the active `CacheBackend` and, if it's `GitHubActionsCache`,
/// restores the engine's persistent state from the cache before any real
/// build work happens. Call this **once**, at the start of a `paws
/// docker`/`paws ci` invocation — not around every individual
/// `core`/`core_streaming` call within it. A single `paws` invocation can
/// call `core` many times (e.g. `paws audit` running several scanners in
/// sequence), and stopping/restoring the shared engine container around
/// each one individually would be both wasteful and unsafe for whatever
/// else might be relying on that engine staying up mid-invocation.
///
/// Always emits the FR-006 "which backend was selected" log line, even
/// when the backend is `None` or `DaggerCloud` (which needs no local
/// restore step of its own — Dagger Cloud caching is transparent to the
/// engine itself once `DAGGER_CLOUD_TOKEN` is set).
///
/// Returns the detected backend so the caller can pass it to
/// [`save_cache_backend`] once the invocation's real pipeline work is
/// done.
pub async fn restore_cache_backend() -> CacheBackend {
    let backend = CacheBackend::detect();
    eprintln!("{}", backend.log_line());
    if let CacheBackend::GitHubActionsCache { base_url, token } = &backend {
        let client = ActionsCacheClient::new(base_url.clone(), token.clone());
        if let Err(err) = restore_github_actions_cache(&client).await {
            eprintln!("cache: restore failed, continuing with a cold build: {err:#}");
        }
    }
    backend
}

/// Saves the engine's persistent state back to `backend`'s cache, if
/// applicable. Call this **once**, after all of the invocation's real
/// `core`/`core_streaming` calls have finished — success or failure;
/// caching whatever state exists is still useful even after a failed
/// build. No-op for `CacheBackend::DaggerCloud`/`CacheBackend::None`.
pub async fn save_cache_backend(backend: &CacheBackend) {
    if let CacheBackend::GitHubActionsCache { base_url, token } = backend {
        let client = ActionsCacheClient::new(base_url.clone(), token.clone());
        if let Err(err) = save_github_actions_cache(&client).await {
            eprintln!("cache: save failed (build result is unaffected): {err:#}");
        }
    }
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

    // Serialized: every test below mutates process-wide env vars
    // (DAGGER_CLOUD_TOKEN/ACTIONS_CACHE_URL/ACTIONS_RUNTIME_TOKEN/
    // ACTIONS_RESULTS_URL), and `cargo test` runs test fns in this module
    // concurrently on separate threads by default — without this, one
    // test's env mutation can leak into another's assertions (matches the
    // existing pattern in paws-environment's tests).
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    const CACHE_ENV_VARS_UNDER_TEST: &[&str] = &[
        "DAGGER_CLOUD_TOKEN",
        "ACTIONS_CACHE_URL",
        "ACTIONS_RUNTIME_TOKEN",
        "ACTIONS_RESULTS_URL",
    ];

    /// SAFETY: guarded by `ENV_LOCK` above — no concurrent env access from
    /// other tests in this module while a guard is held.
    unsafe fn clear_cache_env() {
        for var in CACHE_ENV_VARS_UNDER_TEST {
            unsafe {
                std::env::remove_var(var);
            }
        }
    }

    #[tokio::test]
    async fn detect_prefers_dagger_cloud_when_both_signatures_present() {
        let _guard = ENV_LOCK.lock().await;
        unsafe {
            clear_cache_env();
            std::env::set_var("DAGGER_CLOUD_TOKEN", "dagger-cloud-token-value");
            std::env::set_var("ACTIONS_CACHE_URL", "https://example.invalid/cache");
            std::env::set_var("ACTIONS_RUNTIME_TOKEN", "actions-runtime-token-value");
        }
        assert!(matches!(CacheBackend::detect(), CacheBackend::DaggerCloud));
        unsafe {
            clear_cache_env();
        }
    }

    #[tokio::test]
    async fn detect_picks_github_actions_cache_when_only_that_signature_present() {
        let _guard = ENV_LOCK.lock().await;
        unsafe {
            clear_cache_env();
            std::env::set_var("ACTIONS_CACHE_URL", "https://example.invalid/cache");
            std::env::set_var("ACTIONS_RUNTIME_TOKEN", "actions-runtime-token-value");
        }
        match CacheBackend::detect() {
            CacheBackend::GitHubActionsCache { base_url, token } => {
                assert_eq!(base_url, "https://example.invalid/cache");
                assert_eq!(token, "actions-runtime-token-value");
            }
            other => panic!("expected GitHubActionsCache, got {other:?}"),
        }
        unsafe {
            clear_cache_env();
        }
    }

    #[tokio::test]
    async fn detect_falls_through_to_none_with_no_signatures_present() {
        let _guard = ENV_LOCK.lock().await;
        unsafe {
            clear_cache_env();
        }
        assert!(matches!(CacheBackend::detect(), CacheBackend::None));
    }

    #[tokio::test]
    async fn detect_falls_through_to_none_when_only_actions_results_url_is_set() {
        // FR-007/T014: `$ACTIONS_RESULTS_URL`-only environments (the newer
        // Twirp/protobuf results service this crate doesn't implement) must
        // not be mistaken for `$ACTIONS_CACHE_URL`'s legacy REST API.
        let _guard = ENV_LOCK.lock().await;
        unsafe {
            clear_cache_env();
            std::env::set_var("ACTIONS_RESULTS_URL", "https://example.invalid/results");
        }
        assert!(matches!(CacheBackend::detect(), CacheBackend::None));
        unsafe {
            clear_cache_env();
        }
    }

    #[test]
    fn log_line_names_the_correct_backend_for_each_outcome() {
        assert_eq!(
            CacheBackend::DaggerCloud.log_line(),
            "cache: using dagger-cloud"
        );
        assert_eq!(
            CacheBackend::GitHubActionsCache {
                base_url: "https://example.invalid".to_string(),
                token: "t".to_string(),
            }
            .log_line(),
            "cache: using github-actions"
        );
        assert_eq!(
            CacheBackend::None.log_line(),
            "cache: no backend detected (full rebuild)"
        );
    }

    #[test]
    fn dagger_cloud_token_reaches_the_subprocess_via_inherited_environment() {
        // research.md R6: `DaggerCloud` needs no plumbing beyond detection
        // because `std::process::Command` inherits the full parent
        // environment by default — this only holds as long as nothing in
        // this crate calls `.env_clear()`/`.env_remove()` on the `dagger`
        // command it builds. Characterization test against this file's own
        // source, mirroring how SC-004's lint script mechanically enforces
        // a similar subprocess-hygiene invariant.
        // Only the production code above the test module matters here —
        // slicing it off avoids this assertion's own string literals
        // (which necessarily mention the patterns being checked for)
        // producing a false positive against themselves.
        let source = include_str!("lib.rs");
        let production_code = source
            .split_once("#[cfg(test)]")
            .map(|(before, _)| before)
            .unwrap_or(source);
        assert!(
            !production_code.contains(".env_clear()"),
            "a .env_clear() call would silently break DAGGER_CLOUD_TOKEN propagation"
        );
        assert!(
            !production_code.contains(".env_remove(\"DAGGER_CLOUD_TOKEN\")"),
            "an explicit .env_remove of DAGGER_CLOUD_TOKEN would break research.md R6's guarantee"
        );
    }

    #[tokio::test]
    async fn an_invalid_dagger_cloud_token_never_fails_restore_or_save() {
        // Edge Cases (spec.md): a broken cache backend must never be worse
        // than no cache backend. `DaggerCloud`'s restore/save are no-ops in
        // this crate (Design Decision 4 — near-zero code, the `dagger` CLI
        // itself owns cloud auth success/failure at pipeline-run time, and
        // Dagger Cloud is documented as additive tracing/caching, not a
        // hard pipeline dependency) — so an invalid token can't hard-fail
        // either function, by construction.
        let backend = CacheBackend::DaggerCloud;
        let restored = restore_cache_backend_for_test(&backend).await;
        assert!(matches!(restored, CacheBackend::DaggerCloud));
        save_cache_backend(&backend).await; // must not panic
    }

    /// Test-only helper mirroring `restore_cache_backend`'s body without
    /// re-detecting from the environment, so this test can exercise a
    /// specific `CacheBackend` value directly.
    async fn restore_cache_backend_for_test(backend: &CacheBackend) -> CacheBackend {
        if let CacheBackend::GitHubActionsCache { base_url, token } = backend {
            let client = ActionsCacheClient::new(base_url.clone(), token.clone());
            let _ = restore_github_actions_cache(&client).await;
        }
        backend.clone()
    }

    #[tokio::test]
    async fn actions_cache_client_find_entry_sends_the_correct_request() {
        // T013: fixture test — a minimal local HTTP server standing in for
        // the real Actions Cache Service v1 REST API, asserting
        // `find_entry` builds the correct URL, auth header, and API-version
        // header, and correctly parses a hit response.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind fixture listener");
        let addr = listener.local_addr().unwrap();

        let server = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).into_owned();

            let body = br#"{"archiveLocation":"https://example.invalid/archive.tar.gz"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.write_all(body).await.unwrap();
            request
        });

        let client = ActionsCacheClient::new(format!("http://{addr}"), "fixture-token".to_string());
        let location = client
            .find_entry("fixture-key")
            .await
            .expect("find_entry against the fixture should succeed");
        assert_eq!(
            location.as_deref(),
            Some("https://example.invalid/archive.tar.gz")
        );

        let request = server.await.unwrap();
        assert!(
            request
                .starts_with("GET /_apis/artifactcache/cache?keys=fixture-key&version=fixture-key")
        );
        assert!(
            request.contains("authorization: bearer fixture-token")
                || request
                    .to_lowercase()
                    .contains("authorization: bearer fixture-token")
        );
        assert!(
            request
                .to_lowercase()
                .contains(&format!("api-version={CACHE_API_VERSION}"))
        );
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
