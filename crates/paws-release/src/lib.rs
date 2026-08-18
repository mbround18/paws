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
use std::time::{SystemTime, UNIX_EPOCH};

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
            smoke: Some(SmokeTestSpec { platform: Some("linux/amd64"), image: "ubuntu:24.04", wine: false }),
        },
        TargetConfig {
            triple: "aarch64-unknown-linux-gnu",
            builder_dir: "builders/linux-gnu",
            smoke: Some(SmokeTestSpec { platform: Some("linux/arm64"), image: "ubuntu:24.04", wine: false }),
        },
        TargetConfig {
            triple: "x86_64-unknown-linux-musl",
            builder_dir: "builders/linux-musl-x86_64",
            smoke: Some(SmokeTestSpec { platform: Some("linux/amd64"), image: "alpine:3.20", wine: false }),
        },
        TargetConfig {
            triple: "aarch64-unknown-linux-musl",
            builder_dir: "builders/linux-musl-aarch64",
            smoke: Some(SmokeTestSpec { platform: Some("linux/arm64"), image: "alpine:3.20", wine: false }),
        },
        TargetConfig {
            triple: "x86_64-pc-windows-gnu",
            builder_dir: "builders/windows-gnu",
            smoke: Some(SmokeTestSpec { platform: None, image: "scottyhardy/docker-wine", wine: true }),
        },
        // Mach-O binaries: Dagger can't run a macOS container, and Wine only
        // emulates Windows PE, not Mach-O, so there's no execution
        // environment available to smoke-test these — build+link-verified
        // only (see builders/macos/README.md).
        TargetConfig { triple: "x86_64-apple-darwin", builder_dir: "builders/macos", smoke: None },
        TargetConfig { triple: "aarch64-apple-darwin", builder_dir: "builders/macos", smoke: None },
    ]
}

pub fn target_config(triple: &str) -> Option<TargetConfig> {
    known_targets().into_iter().find(|t| t.triple == triple)
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
    /// Stamped onto the builder image as `org.opencontainers.image.version`.
    pub builder_version: &'a str,
    /// Stamped onto the builder image as `org.opencontainers.image.revision`.
    pub builder_revision: &'a str,
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
    let name = Path::new(builder_dir).file_name().and_then(|s| s.to_str()).unwrap_or(builder_dir);
    format!("ghcr.io/mbround18/paws-builders:{name}-{version}")
}

/// Builds `./<builder_dir>/Dockerfile` (via `dagger core ... docker-build`,
/// getting Dagger's own BuildKit layer caching for free — no separate cache
/// setup needed), mounts `source_dir` into it, runs `cargo build --release
/// --target <triple> -p <package>` inside it, and exports the resulting
/// binary to `target/dagger-release/<triple>/<binary_name>`.
///
/// Tries [`prebuilt_image_candidate`] first (`container from --address=...`)
/// via `paws_dagger::remote_image_exists` — when `release.yaml`'s
/// `build-builders` job already pushed this exact builder+version, this
/// skips paying for the Dockerfile's own build from scratch. Falls back to
/// the local `docker-build` path whenever it isn't there (a fresh builder,
/// a version nothing has pushed for yet, or running outside CI entirely).
pub async fn build_binary(request: &BuildRequest<'_>) -> Result<PathBuf> {
    let file_name = binary_file_name(request.binary_name, request.triple);
    let container_bin_path = format!("target/{}/release/{}", request.triple, file_name);

    let out_dir = Path::new("target").join("dagger-release").join(request.triple);
    tokio::fs::create_dir_all(&out_dir).await.context("failed to create release output directory")?;
    let out_path = out_dir.join(&file_name);

    let created_unix = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let build_args = format!(
        "BUILDER_VERSION={},BUILDER_REVISION={},BUILDER_CREATED={created_unix}",
        request.builder_version, request.builder_revision
    );

    let prebuilt = prebuilt_image_candidate(request.builder_dir, request.builder_version);
    let mut args: Vec<String> = if paws_dagger::remote_image_exists(&prebuilt).await {
        vec!["container".into(), "from".into(), format!("--address={prebuilt}")]
    } else {
        vec![
            "host".into(),
            "directory".into(),
            format!("--path={}", request.builder_dir),
            "docker-build".into(),
            format!("--build-args={build_args}"),
        ]
    };

    args.extend([
        "with-mounted-directory".into(),
        "--path=/src".into(),
        format!("--source={}", request.source_dir),
        "with-workdir".into(),
        "--path=/src".into(),
        "with-exec".into(),
        format!("--args=cargo,build,--release,--target,{},-p,{}", request.triple, request.package),
        "file".into(),
        format!("--path={container_bin_path}"),
        "export".into(),
        format!("--path={}", out_path.display()),
    ]);

    paws_dagger::core(&args).await.with_context(|| format!("dagger build failed for target {}", request.triple))?;
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
        args.extend(["with-exec".into(), format!("--args=wine,{container_path},--version")]);
    } else {
        args.extend(["with-exec".into(), format!("--args=chmod,+x,{container_path}")]);
        args.extend(["with-exec".into(), format!("--args={container_path},--version")]);
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
        tokio::fs::create_dir_all(parent).await.context("failed to create archive output directory")?;
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
        Self { owner, repo, token, client: reqwest::Client::new() }
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

    /// Finds the release for `tag`, creating it (as a prerelease if
    /// `prerelease` is true) if it doesn't exist yet. Returns the release id.
    pub async fn get_or_create_release(&self, tag: &str, prerelease: bool) -> Result<u64> {
        let get_url = format!("{}/releases/tags/{tag}", self.api_base());
        let response = self.auth_headers(self.client.get(&get_url)).send().await.context("failed to query release by tag")?;

        if response.status().is_success() {
            let body: serde_json::Value = response.json().await.context("failed to parse release response")?;
            return body.get("id").and_then(|v| v.as_u64()).context("release response missing id");
        }
        if response.status() != reqwest::StatusCode::NOT_FOUND {
            anyhow::bail!("unexpected status querying release: {}", response.status());
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

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("failed to create release for tag {tag}: {status}: {text}");
        }
        let body: serde_json::Value = response.json().await.context("failed to parse created-release response")?;
        body.get("id").and_then(|v| v.as_u64()).context("created-release response missing id")
    }

    /// Uploads `file_path` as a release asset, replacing any existing asset
    /// with the same name first (mirrors `gh release upload --clobber`).
    pub async fn upload_asset(&self, release_id: u64, file_path: &Path) -> Result<()> {
        let file_name =
            file_path.file_name().and_then(|n| n.to_str()).context("archive path has no file name")?.to_string();

        self.delete_existing_asset(release_id, &file_name).await?;

        let bytes = tokio::fs::read(file_path).await.context("failed to read archive for upload")?;
        let upload_url =
            format!("https://uploads.github.com/repos/{}/{}/releases/{release_id}/assets?name={file_name}", self.owner, self.repo);

        let response = self
            .auth_headers(self.client.post(&upload_url))
            .header("Content-Type", "application/zip")
            .body(bytes)
            .send()
            .await
            .context("failed to upload release asset")?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("failed to upload asset {file_name}: {status}: {text}");
        }
        Ok(())
    }

    async fn delete_existing_asset(&self, release_id: u64, file_name: &str) -> Result<()> {
        let list_url = format!("{}/releases/{release_id}/assets", self.api_base());
        let response =
            self.auth_headers(self.client.get(&list_url)).send().await.context("failed to list release assets")?;
        if !response.status().is_success() {
            return Ok(()); // best-effort; the upload itself will surface a clearer error if this matters
        }
        let assets: Vec<serde_json::Value> = response.json().await.unwrap_or_default();
        let Some(existing) = assets.iter().find(|a| a.get("name").and_then(|n| n.as_str()) == Some(file_name)) else {
            return Ok(());
        };
        let Some(asset_id) = existing.get("id").and_then(|v| v.as_u64()) else { return Ok(()) };

        let delete_url = format!("{}/releases/assets/{asset_id}", self.api_base());
        self.auth_headers(self.client.delete(&delete_url)).send().await.context("failed to delete existing asset")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_targets_get_exe_suffix() {
        assert_eq!(binary_file_name("paws", "x86_64-pc-windows-gnu"), "paws.exe");
        assert_eq!(binary_file_name("paws", "x86_64-unknown-linux-gnu"), "paws");
        assert_eq!(binary_file_name("paws", "x86_64-unknown-linux-musl"), "paws");
        assert_eq!(binary_file_name("paws", "aarch64-unknown-linux-gnu"), "paws");
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
            let dockerfile = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(target.builder_dir).join("Dockerfile");
            assert!(dockerfile.is_file(), "missing {dockerfile:?} for target {}", target.triple);
        }
    }

    #[tokio::test]
    async fn package_zip_produces_a_flat_archive_with_the_binary() {
        let dir = std::env::temp_dir().join(format!("paws-release-test-{}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let nested = dir.join("nested");
        tokio::fs::create_dir_all(&nested).await.unwrap();
        tokio::fs::write(nested.join("paws"), b"fake binary contents").await.unwrap();

        let archive_path = dir.join("out").join("paws-test.zip");
        package_zip(&dir, &archive_path, &["nested/paws".to_string()]).await.unwrap();

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
        assert!(!listing.contains("nested/paws"), "expected a flat archive, got: {listing}");

        tokio::fs::remove_dir_all(&dir).await.ok();
    }
}
