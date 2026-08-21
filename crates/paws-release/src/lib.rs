//! Cross-target binary release: build, smoke-test, package, and publish to a
//! GitHub Release. Dogfoods `paws` on itself — the same binary that runs
//! `paws ci` also runs `paws release` in this repo's own release workflow.
//!
//! Building and smoke-testing both go through `paws-dagger::core` (moduleless
//! `dagger core <chain>` pipelines against `./builders/*` Dockerfiles) —
//! never a direct `docker`/`cross` spawn. That keeps `paws-dagger` the single
//! seam that talks to a container engine, gives every build Dagger's own
//! BuildKit layer caching for free, and means a user running `paws release`
//! only ever needs the `dagger` CLI, not Docker/`cross`/QEMU/Wine set up
//! independently — Dagger's own `--platform` support covers cross-arch
//! execution (backed by the host's QEMU `binfmt_misc` registration), and a
//! Wine-enabled base image covers the Windows target the same way.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tokio::process::Command;

/// A build target: which `./builders/<dir>` Dockerfile builds it, and how to
/// smoke-test the resulting binary. `smoke: None` means "build/link-verified
/// only" — no execution environment for this target's binaries is available
/// (e.g. macOS/Mach-O: Dagger can't run Mach-O containers, and Wine only
/// handles Windows PE, not Mach-O).
#[derive(Debug, Clone)]
pub struct TargetConfig {
    pub triple: &'static str,
    pub builder_dir: &'static str,
    pub smoke: Option<SmokeTestSpec>,
}

/// How to actually execute a built binary to prove it runs, not just that it
/// compiled. `platform` drives Dagger's own cross-arch container execution;
/// `wine` runs the binary under Wine inside the container instead of
/// executing it directly (for the Windows target, which Dagger can't run as
/// a native container).
#[derive(Debug, Clone)]
pub struct SmokeTestSpec {
    pub platform: Option<&'static str>,
    pub image: &'static str,
    pub wine: bool,
}

/// The known target matrix. `README.md`/`builders/README.md` document the
/// same set; keep them in sync when adding a target.
pub fn known_targets() -> Vec<TargetConfig> {
    vec![
        TargetConfig {
            triple: "x86_64-unknown-linux-gnu",
            builder_dir: "builders/linux-gnu",
            smoke: Some(SmokeTestSpec {
                platform: Some("linux/amd64"),
                image: "ubuntu:24.04",
                wine: false,
            }),
        },
        TargetConfig {
            triple: "aarch64-unknown-linux-gnu",
            builder_dir: "builders/linux-gnu",
            smoke: Some(SmokeTestSpec {
                platform: Some("linux/arm64"),
                image: "ubuntu:24.04",
                wine: false,
            }),
        },
        TargetConfig {
            triple: "x86_64-unknown-linux-musl",
            builder_dir: "builders/linux-musl-x86_64",
            smoke: Some(SmokeTestSpec {
                platform: Some("linux/amd64"),
                image: "alpine:3.20",
                wine: false,
            }),
        },
        TargetConfig {
            triple: "aarch64-unknown-linux-musl",
            builder_dir: "builders/linux-musl-aarch64",
            smoke: Some(SmokeTestSpec {
                platform: Some("linux/arm64"),
                image: "alpine:3.20",
                wine: false,
            }),
        },
        TargetConfig {
            triple: "x86_64-pc-windows-gnu",
            builder_dir: "builders/windows-gnu",
            smoke: Some(SmokeTestSpec {
                platform: None,
                image: "scottyhardy/docker-wine",
                wine: true,
            }),
        },
        // Mach-O binaries: Dagger can't run a macOS container, and Wine only
        // emulates Windows PE, not Mach-O, so there's no execution
        // environment available to smoke-test these — build+link-verified
        // only (see builders/macos/README.md).
        TargetConfig {
            triple: "x86_64-apple-darwin",
            builder_dir: "builders/macos",
            smoke: None,
        },
        TargetConfig {
            triple: "aarch64-apple-darwin",
            builder_dir: "builders/macos",
            smoke: None,
        },
    ]
}

pub fn target_config(triple: &str) -> Option<TargetConfig> {
    known_targets().into_iter().find(|t| t.triple == triple)
}

/// The generic Rust Linux builder Dockerfile, embedded at compile time from
/// `builders/linux-gnu/Dockerfile` — the same Dockerfile `known_targets()`
/// uses for paws's own `x86_64-unknown-linux-gnu`/`aarch64-unknown-linux-gnu`
/// legs, but reused here via [`write_generic_builder_dockerfile`] +
/// [`build_binary_local`] so a target repo with no `builders/` directory of
/// its own (e.g. `ark-manager-web`) can still run `paws release
/// --local-build`. Mirrors `paws-tauri`'s embed-and-materialize pattern —
/// see that crate's `TAURI_LINUX_DOCKERFILE` for why embedding beats a
/// repo-relative path: `paws release` runs from inside whatever repo it's
/// releasing, not from inside `paws`'s own source tree.
const GENERIC_LINUX_GNU_DOCKERFILE: &str = include_str!("../../../builders/linux-gnu/Dockerfile");

/// Targets [`build_binary_local`] can build, i.e. ones the embedded
/// [`GENERIC_LINUX_GNU_DOCKERFILE`] actually has a toolchain for. Deliberately
/// narrow (linux-gnu only, no macOS/Windows) — a generic cross matrix is
/// speculative until a second target repo actually needs it.
pub fn local_build_targets() -> &'static [&'static str] {
    &["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"]
}

/// Writes the embedded generic Linux builder Dockerfile to a temp directory
/// and returns that directory's path, suitable for [`build_binary_local`].
pub fn write_generic_builder_dockerfile() -> Result<PathBuf> {
    let dir = std::env::temp_dir()
        .join("paws-builders")
        .join("generic-linux-gnu");
    std::fs::create_dir_all(&dir)
        .context("failed to create temp dir for the generic linux-gnu builder Dockerfile")?;
    std::fs::write(dir.join("Dockerfile"), GENERIC_LINUX_GNU_DOCKERFILE)
        .context("failed to write the generic linux-gnu builder Dockerfile")?;
    Ok(dir)
}

/// Windows targets produce `<name>.exe`; every other target produces `<name>`.
pub fn binary_file_name(binary_name: &str, target: &str) -> String {
    if target.contains("windows") {
        format!("{binary_name}.exe")
    } else {
        binary_name.to_string()
    }
}

/// Conventional release-asset archive name: `<name>-<version>-<target>.zip`.
pub fn archive_name(binary_name: &str, version: &str, target: &str) -> String {
    format!("{binary_name}-{version}-{target}.zip")
}

/// Inputs for [`build_binary`].
pub struct BuildRequest<'a> {
    pub builder_dir: &'a str,
    /// Host path to the source tree to build (mounted read-write at `/src`).
    pub source_dir: &'a str,
    pub triple: &'a str,
    pub package: &'a str,
    pub binary_name: &'a str,
    /// Version tag the prebuilt builder image was pushed under (see
    /// [`prebuilt_image_candidate`]) — normally the release tag itself.
    pub builder_version: &'a str,
}

/// The prebuilt registry image `compose.yml`/`release.yaml`'s
/// `build-builders` job would have pushed for `builder_dir`/`version`, e.g.
/// `builders/linux-gnu` + `v0.0.1-prerelease.1` ->
/// `ghcr.io/mbround18/paws-builders:linux-gnu-v0.0.1-prerelease.1`. Flat
/// repo + `<builder>-<version>` tag, not one repo per builder — Docker
/// Hub doesn't support nested repository paths, so `compose.yml` uses the
/// same flat scheme on both registries this candidate needs to match. Pure
/// string construction, checked against the registry separately (see
/// [`build_binary`]) — this function can't know whether the image actually
/// exists.
pub fn prebuilt_image_candidate(builder_dir: &str, version: &str) -> String {
    let name = Path::new(builder_dir)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(builder_dir);
    format!("ghcr.io/mbround18/paws-builders:{name}-{version}")
}

/// Pulls the prebuilt builder image ([`prebuilt_image_candidate`]) —
/// `release.yaml`'s `build-builders` job pushes it before the target
/// matrix starts (`needs: [ci, build-builders]`), so it's always there by
/// the time this runs in CI — mounts `source_dir` into it, runs `cargo
/// build --release --target <triple> -p <package>` inside it, and exports
/// the resulting binary to `target/dagger-release/<triple>/<binary_name>`.
///
/// Deliberately pull-only, no local `docker-build` fallback: building
/// `./<builder_dir>/Dockerfile` from scratch here would mean every target
/// leg re-pays for a build `build-builders` already did once, the exact
/// duplicated cost prebuilt images exist to remove, and it would silently
/// mask a `build-builders` failure instead of surfacing it. Anything that
/// isn't paws's own release pipeline (a Dockerfile-less `builder_dir`, or
/// this exact version never pushed) fails loudly here instead.
pub async fn build_binary(request: &BuildRequest<'_>) -> Result<PathBuf> {
    let file_name = binary_file_name(request.binary_name, request.triple);
    let container_bin_path = format!("target/{}/release/{}", request.triple, file_name);

    let out_dir = Path::new("target")
        .join("dagger-release")
        .join(request.triple);
    tokio::fs::create_dir_all(&out_dir)
        .await
        .context("failed to create release output directory")?;
    let out_path = out_dir.join(&file_name);

    let prebuilt = prebuilt_image_candidate(request.builder_dir, request.builder_version);
    if !paws_dagger::remote_image_exists_with_retry(&prebuilt).await {
        anyhow::bail!(
            "prebuilt builder image {prebuilt} not found - push it first (docker compose build \
             && docker compose push against ./compose.yml, or let release.yaml's build-builders \
             job do it) before running `paws release --target {}`",
            request.triple
        );
    }

    let mut args: Vec<String> = vec![
        "container".into(),
        "from".into(),
        format!("--address={prebuilt}"),
    ];

    args.extend([
        "with-mounted-directory".into(),
        "--path=/src".into(),
        format!("--source={}", request.source_dir),
        "with-workdir".into(),
        "--path=/src".into(),
        "with-exec".into(),
        format!(
            "--args=cargo,build,--release,--target,{},-p,{}",
            request.triple, request.package
        ),
        "file".into(),
        format!("--path={container_bin_path}"),
        "export".into(),
        format!("--path={}", out_path.display()),
    ]);

    paws_dagger::core(&args)
        .await
        .with_context(|| format!("dagger build failed for target {}", request.triple))?;
    Ok(out_path)
}

/// Same contract as [`build_binary`], but for repos outside `paws`'s own —
/// ones with no `builders/` directory and no prebuilt `paws-builders` image
/// to pull. Builds `builder_dir` (normally
/// [`write_generic_builder_dockerfile`]'s output) locally via Dagger's
/// `docker-build` (mirrors `paws-tauri::dagger_pipeline_args`'s
/// `host directory --path=<dir> docker-build` chain) instead of pulling a
/// prebuilt image — the one-time Dockerfile build cost that model exists to
/// avoid inside `paws`'s own release pipeline doesn't apply here, since
/// there's no `build-builders` job in a target repo to have paid it already.
/// `request.builder_dir` is ignored in favor of `local_builder_dir` — kept
/// as a separate parameter (rather than repurposing the field) so callers
/// can't accidentally pass a paws-relative `builder_dir` here by habit.
pub async fn build_binary_local(
    request: &BuildRequest<'_>,
    local_builder_dir: &Path,
) -> Result<PathBuf> {
    anyhow::ensure!(
        local_build_targets().contains(&request.triple),
        "--local-build only supports {}; got {}",
        local_build_targets().join(", "),
        request.triple
    );

    let file_name = binary_file_name(request.binary_name, request.triple);
    let container_bin_path = format!("target/{}/release/{}", request.triple, file_name);

    let out_dir = Path::new("target")
        .join("dagger-release")
        .join(request.triple);
    tokio::fs::create_dir_all(&out_dir)
        .await
        .context("failed to create release output directory")?;
    let out_path = out_dir.join(&file_name);

    let args: Vec<String> = vec![
        "host".into(),
        "directory".into(),
        format!("--path={}", local_builder_dir.display()),
        "docker-build".into(),
        "with-mounted-directory".into(),
        "--path=/src".into(),
        format!("--source={}", request.source_dir),
        "with-workdir".into(),
        "--path=/src".into(),
        "with-exec".into(),
        format!(
            "--args=cargo,build,--release,--target,{},-p,{}",
            request.triple, request.package
        ),
        "file".into(),
        format!("--path={container_bin_path}"),
        "export".into(),
        format!("--path={}", out_path.display()),
    ];

    paws_dagger::core(&args)
        .await
        .with_context(|| format!("dagger local build failed for target {}", request.triple))?;
    Ok(out_path)
}

/// Runs `binary_path --version` (or `wine <path> --version` for
/// [`SmokeTestSpec::wine`]) inside a Dagger container — `spec.platform`
/// drives cross-arch execution via Dagger's own QEMU-backed platform
/// support, never a manual `docker run --platform`/`cross`/`qemu-user`
/// setup. Returns the captured stdout so callers can assert on it.
pub async fn smoke_test(binary_path: &Path, spec: &SmokeTestSpec) -> Result<String> {
    let mut args: Vec<String> = vec!["container".into()];
    if let Some(platform) = spec.platform {
        args.push(format!("--platform={platform}"));
    }
    args.push("from".into());
    args.push(format!("--address={}", spec.image));

    let container_path = if spec.wine { "/work/paws.exe" } else { "/paws" };
    args.extend([
        "with-mounted-file".into(),
        format!("--path={container_path}"),
        format!("--source={}", binary_path.display()),
    ]);

    if spec.wine {
        args.extend([
            "with-exec".into(),
            format!("--args=wine,{container_path},--version"),
        ]);
    } else {
        args.extend([
            "with-exec".into(),
            format!("--args=chmod,+x,{container_path}"),
        ]);
        args.extend([
            "with-exec".into(),
            format!("--args={container_path},--version"),
        ]);
    }
    args.push("stdout".into());

    paws_dagger::core(&args).await.context("smoke test failed")
}

/// Zips `files` (paths relative to `working_dir`) into `archive_path`, junking
/// their directory structure (flat archive: just the binary at the top
/// level). Shells to the system `zip` — a plain host utility, not a second
/// build/execution backend, so it stays outside the Dagger-routing concern
/// `build_binary`/`smoke_test` exist to satisfy.
pub async fn package_zip(working_dir: &Path, archive_path: &Path, files: &[String]) -> Result<()> {
    if let Some(parent) = archive_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .context("failed to create archive output directory")?;
    }

    let output = Command::new("zip")
        .current_dir(working_dir)
        .arg("-j") // junk paths: flatten into the archive root
        .arg(archive_path)
        .args(files)
        .output()
        .await
        .context("failed to spawn `zip` — is it installed and on PATH?")?;

    if !output.status.success() {
        anyhow::bail!("`zip` failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(())
}

/// Whether a `POST /releases`' `422 Unprocessable Entity` body is GitHub's
/// specific "a release for this tag already exists" error, as opposed to
/// some other validation failure — checked as a plain substring rather than
/// parsing the full `{"errors": [{"code": ..., "field": ...}]}` shape,
/// since `code":"already_exists"` combined with `field":"tag_name"` is
/// specific enough not to false-positive on an unrelated 422, and doesn't
/// require pinning to GitHub's exact error-array field ordering.
fn is_tag_already_exists_error(response_body: &str) -> bool {
    response_body.contains("\"code\":\"already_exists\"") && response_body.contains("\"tag_name\"")
}

/// Minimal GitHub REST API client for release publishing: get-or-create a
/// release for a tag, then upload (or replace) an asset on it. Plain HTTPS
/// calls, not a process spawn, so this isn't part of the Dagger-routing
/// concern either.
pub struct GitHubReleaseClient {
    pub owner: String,
    pub repo: String,
    pub token: String,
    client: reqwest::Client,
}

impl GitHubReleaseClient {
    pub fn new(owner: String, repo: String, token: String) -> Self {
        Self {
            owner,
            repo,
            token,
            client: reqwest::Client::new(),
        }
    }

    fn api_base(&self) -> String {
        format!("https://api.github.com/repos/{}/{}", self.owner, self.repo)
    }

    fn auth_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .bearer_auth(&self.token)
            .header("User-Agent", "paws-release")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
    }

    /// Looks up the release for `tag`, returning `None` on a 404 (no release
    /// yet) rather than erroring — callers decide what a miss means.
    async fn fetch_release_by_tag(&self, tag: &str) -> Result<Option<u64>> {
        let get_url = format!("{}/releases/tags/{tag}", self.api_base());
        let response = self
            .auth_headers(self.client.get(&get_url))
            .send()
            .await
            .context("failed to query release by tag")?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            anyhow::bail!("unexpected status querying release: {}", response.status());
        }
        let body: serde_json::Value = response
            .json()
            .await
            .context("failed to parse release response")?;
        body.get("id")
            .and_then(|v| v.as_u64())
            .context("release response missing id")
            .map(Some)
    }

    /// Finds the release for `tag`, creating it (as a prerelease if
    /// `prerelease` is true) if it doesn't exist yet. Returns the release id.
    ///
    /// Handles the race a parallel per-target release matrix (`release.yaml`)
    /// creates for real: two legs can both see "no release yet" and both
    /// attempt to create one — GitHub accepts the first and rejects the
    /// second with `422 Unprocessable Entity` /
    /// `{"code":"already_exists","field":"tag_name"}` (verified for real
    /// against a genuine `v0.0.1-prerelease.16` run: the `linux-gnu` and
    /// `linux-musl-x86_64` legs raced, `linux-musl-x86_64` lost and failed
    /// the whole job instead of just fetching the release the other leg had
    /// just created). Rather than treat that 422 as fatal, re-fetch by tag —
    /// the losing leg still gets a valid release id to upload its asset to.
    pub async fn get_or_create_release(&self, tag: &str, prerelease: bool) -> Result<u64> {
        if let Some(id) = self.fetch_release_by_tag(tag).await? {
            return Ok(id);
        }

        let create_url = format!("{}/releases", self.api_base());
        let body = serde_json::json!({
            "tag_name": tag,
            "name": tag,
            "prerelease": prerelease,
            "generate_release_notes": true,
        });
        let response = self
            .auth_headers(self.client.post(&create_url))
            .json(&body)
            .send()
            .await
            .context("failed to create release")?;

        if response.status() == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
            let text = response.text().await.unwrap_or_default();
            if is_tag_already_exists_error(&text) {
                return self.fetch_release_by_tag(tag).await?.with_context(|| {
                    format!(
                        "release for tag {tag} reported as already existing, but a \
                         follow-up fetch found nothing"
                    )
                });
            }
            anyhow::bail!(
                "failed to create release for tag {tag}: 422 Unprocessable Entity: {text}"
            );
        }
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("failed to create release for tag {tag}: {status}: {text}");
        }
        let body: serde_json::Value = response
            .json()
            .await
            .context("failed to parse created-release response")?;
        body.get("id")
            .and_then(|v| v.as_u64())
            .context("created-release response missing id")
    }

    /// Uploads `file_path` as a release asset, replacing any existing asset
    /// with the same name first (mirrors `gh release upload --clobber`).
    /// Thin wrapper over [`upload_asset_with`](Self::upload_asset_with) —
    /// kept as the stable public entry point `paws release` already calls.
    pub async fn upload_asset(&self, release_id: u64, file_path: &Path) -> Result<()> {
        self.upload_asset_with(
            release_id,
            file_path,
            "application/zip",
            AssetUploadMode::Clobber,
        )
        .await
        .map(|_uploaded| ())
    }

    /// Same as [`upload_asset`](Self::upload_asset), but lets the caller
    /// pick the upload `Content-Type` and whether a same-named existing
    /// asset gets replaced ([`AssetUploadMode::Clobber`], `paws release`'s
    /// binaries — a re-run of the same tag should replace them) or left
    /// alone ([`AssetUploadMode::SkipIfExisting`], `paws helm --publish`'s
    /// chart packages — a previously-published version must never change
    /// underneath its already-recorded `index.yaml` digest). Returns
    /// whether it actually uploaded (`false` under `SkipIfExisting` means
    /// "already there, left as-is").
    pub async fn upload_asset_with(
        &self,
        release_id: u64,
        file_path: &Path,
        content_type: &str,
        mode: AssetUploadMode,
    ) -> Result<bool> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .context("archive path has no file name")?
            .to_string();

        if let Some(asset_id) = self.find_existing_asset_id(release_id, &file_name).await? {
            match mode {
                AssetUploadMode::SkipIfExisting => return Ok(false),
                AssetUploadMode::Clobber => {
                    let delete_url = format!("{}/releases/assets/{asset_id}", self.api_base());
                    self.auth_headers(self.client.delete(&delete_url))
                        .send()
                        .await
                        .context("failed to delete existing asset")?;
                }
            }
        }

        let bytes = tokio::fs::read(file_path)
            .await
            .context("failed to read archive for upload")?;
        let upload_url = format!(
            "https://uploads.github.com/repos/{}/{}/releases/{release_id}/assets?name={file_name}",
            self.owner, self.repo
        );

        let response = self
            .auth_headers(self.client.post(&upload_url))
            .header("Content-Type", content_type)
            .body(bytes)
            .send()
            .await
            .context("failed to upload release asset")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("failed to upload asset {file_name}: {status}: {text}");
        }
        Ok(true)
    }

    /// Looks up an existing asset named `file_name` on `release_id`,
    /// returning its id — best-effort: a failure to even list assets is
    /// treated as "none found" rather than erroring, since the upload
    /// itself will surface a clearer error if that turns out to matter.
    async fn find_existing_asset_id(
        &self,
        release_id: u64,
        file_name: &str,
    ) -> Result<Option<u64>> {
        let list_url = format!("{}/releases/{release_id}/assets", self.api_base());
        let response = self
            .auth_headers(self.client.get(&list_url))
            .send()
            .await
            .context("failed to list release assets")?;
        if !response.status().is_success() {
            return Ok(None);
        }
        let assets: Vec<serde_json::Value> = response.json().await.unwrap_or_default();
        Ok(assets
            .iter()
            .find(|a| a.get("name").and_then(|n| n.as_str()) == Some(file_name))
            .and_then(|a| a.get("id"))
            .and_then(|v| v.as_u64()))
    }

    /// Fetches `path` at `git_ref` via the Contents API — decoded file
    /// bytes plus the blob `sha` a follow-up [`put_content`](Self::put_content)
    /// needs to update it in place. `None` on a 404 (no such file at that
    /// ref yet — e.g. a chart repo's first-ever publish, before
    /// `index.yaml` exists on the pages branch at all).
    pub async fn get_content(&self, path: &str, git_ref: &str) -> Result<Option<ContentFile>> {
        use base64::Engine;

        let url = format!("{}/contents/{path}?ref={git_ref}", self.api_base());
        let response = self
            .auth_headers(self.client.get(&url))
            .send()
            .await
            .context("failed to fetch file content")?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            anyhow::bail!(
                "unexpected status fetching {path}@{git_ref}: {}",
                response.status()
            );
        }

        let body: serde_json::Value = response
            .json()
            .await
            .context("failed to parse file content response")?;
        let sha = body
            .get("sha")
            .and_then(|v| v.as_str())
            .context("file content response missing sha")?
            .to_string();
        // The Contents API's `content` field is base64 with embedded
        // newlines (wrapped for readability) - strip them before decoding.
        let encoded: String = body
            .get("content")
            .and_then(|v| v.as_str())
            .context("file content response missing content")?
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let content = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .context("failed to decode file content")?;
        Ok(Some(ContentFile { content, sha }))
    }

    /// Creates or updates `path` on `branch` via the Contents API. Pass the
    /// `sha` from a prior [`get_content`](Self::get_content) call to update
    /// an existing file in place; omit it (`None`) only when the file is
    /// known not to exist yet — GitHub rejects a create-with-sha or an
    /// update-without-sha with a 409/422, so getting this wrong surfaces
    /// immediately rather than silently corrupting anything.
    pub async fn put_content(
        &self,
        path: &str,
        branch: &str,
        content: &[u8],
        message: &str,
        sha: Option<&str>,
    ) -> Result<()> {
        use base64::Engine;

        let url = format!("{}/contents/{path}", self.api_base());
        let mut body = serde_json::json!({
            "message": message,
            "content": base64::engine::general_purpose::STANDARD.encode(content),
            "branch": branch,
        });
        if let Some(sha) = sha {
            body["sha"] = serde_json::Value::String(sha.to_string());
        }

        let response = self
            .auth_headers(self.client.put(&url))
            .json(&body)
            .send()
            .await
            .context("failed to publish file content")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("failed to publish {path} to {branch}: {status}: {text}");
        }
        Ok(())
    }
}

/// A file fetched via [`GitHubReleaseClient::get_content`].
pub struct ContentFile {
    pub content: Vec<u8>,
    pub sha: String,
}

/// Whether [`GitHubReleaseClient::upload_asset_with`] replaces a same-named
/// existing asset or leaves it alone — see that method's doc comment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetUploadMode {
    Clobber,
    SkipIfExisting,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_real_tag_already_exists_error_body() {
        // Captured verbatim from a real race: v0.0.1-prerelease.16's
        // linux-gnu and linux-musl-x86_64 release.yaml legs both tried to
        // create the same tag's release; linux-musl-x86_64 lost with
        // exactly this body.
        let body = r#"{"message":"Validation Failed","errors":[{"resource":"Release","code":"already_exists","field":"tag_name"}],"documentation_url":"https://docs.github.com/rest/releases/releases#create-a-release","status":"422"}"#;
        assert!(is_tag_already_exists_error(body));
    }

    #[test]
    fn does_not_misclassify_an_unrelated_422() {
        let body = r#"{"message":"Validation Failed","errors":[{"resource":"Release","code":"invalid","field":"name"}]}"#;
        assert!(!is_tag_already_exists_error(body));
        assert!(!is_tag_already_exists_error(""));
    }

    #[test]
    fn windows_targets_get_exe_suffix() {
        assert_eq!(
            binary_file_name("paws", "x86_64-pc-windows-gnu"),
            "paws.exe"
        );
        assert_eq!(binary_file_name("paws", "x86_64-unknown-linux-gnu"), "paws");
        assert_eq!(
            binary_file_name("paws", "x86_64-unknown-linux-musl"),
            "paws"
        );
        assert_eq!(
            binary_file_name("paws", "aarch64-unknown-linux-gnu"),
            "paws"
        );
    }

    #[test]
    fn archive_name_follows_convention() {
        assert_eq!(
            archive_name("paws", "0.0.1-prerelease.1", "x86_64-unknown-linux-gnu"),
            "paws-0.0.1-prerelease.1-x86_64-unknown-linux-gnu.zip"
        );
    }

    #[test]
    fn prebuilt_image_candidate_uses_the_builder_dir_basename() {
        assert_eq!(
            prebuilt_image_candidate("builders/linux-gnu", "v0.0.1-prerelease.1"),
            "ghcr.io/mbround18/paws-builders:linux-gnu-v0.0.1-prerelease.1"
        );
        assert_eq!(
            prebuilt_image_candidate("builders/macos", "v1.2.3"),
            "ghcr.io/mbround18/paws-builders:macos-v1.2.3"
        );
    }

    #[test]
    fn known_targets_each_have_a_matching_builder_dockerfile() {
        for target in known_targets() {
            let dockerfile = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(target.builder_dir)
                .join("Dockerfile");
            assert!(
                dockerfile.is_file(),
                "missing {dockerfile:?} for target {}",
                target.triple
            );
        }
    }

    #[test]
    fn write_generic_builder_dockerfile_materializes_the_embedded_dockerfile() {
        let dir = write_generic_builder_dockerfile().unwrap();
        let contents = std::fs::read_to_string(dir.join("Dockerfile")).unwrap();
        assert_eq!(contents, GENERIC_LINUX_GNU_DOCKERFILE);
    }

    #[test]
    fn generic_builder_dockerfile_matches_the_linux_gnu_builder() {
        // Deliberately the same file (see GENERIC_LINUX_GNU_DOCKERFILE's doc
        // comment) — this pins that assumption rather than letting the two
        // silently drift apart.
        let dockerfile = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("builders/linux-gnu/Dockerfile");
        let contents = std::fs::read_to_string(dockerfile).unwrap();
        assert_eq!(contents, GENERIC_LINUX_GNU_DOCKERFILE);
    }

    #[tokio::test]
    async fn build_binary_local_rejects_unsupported_targets() {
        let request = BuildRequest {
            builder_dir: "unused",
            source_dir: ".",
            triple: "x86_64-apple-darwin",
            package: "paws-cli",
            binary_name: "paws",
            builder_version: "v0.0.0",
        };
        let err = build_binary_local(&request, Path::new("/tmp/does-not-matter"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("--local-build only supports"));
    }

    #[tokio::test]
    async fn package_zip_produces_a_flat_archive_with_the_binary() {
        // `zip`/`unzip` aren't guaranteed to be on PATH (e.g. a bare
        // `rust:1-bookworm` container, which `paws-rust`'s own native CI
        // pipeline uses) — skip rather than fail when they're genuinely
        // absent, the same convention `paws-dagger`'s tests use for the
        // `dagger` CLI.
        if tokio::process::Command::new("zip")
            .arg("--version")
            .output()
            .await
            .is_err()
        {
            return;
        }

        let dir = std::env::temp_dir().join(format!("paws-release-test-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let nested = dir.join("nested");
        tokio::fs::create_dir_all(&nested).await.unwrap();
        tokio::fs::write(nested.join("paws"), b"fake binary contents")
            .await
            .unwrap();

        let archive_path = dir.join("out").join("paws-test.zip");
        package_zip(&dir, &archive_path, &["nested/paws".to_string()])
            .await
            .unwrap();

        assert!(archive_path.is_file());

        // Unzip and verify the binary landed at the archive root (flattened),
        // not nested under "nested/".
        let list_output = tokio::process::Command::new("unzip")
            .arg("-l")
            .arg(&archive_path)
            .output()
            .await
            .unwrap();
        let listing = String::from_utf8_lossy(&list_output.stdout);
        assert!(listing.contains("paws"), "listing: {listing}");
        assert!(
            !listing.contains("nested/paws"),
            "expected a flat archive, got: {listing}"
        );

        tokio::fs::remove_dir_all(&dir).await.ok();
    }
}
