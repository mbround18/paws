//! Flatpak project detection and CI pipeline construction. Verified for
//! real, end to end, against a genuine app (mbround18/oled-wallpaper's real
//! Flatpak manifest, a heavy wgpu/winit GUI app) — not a synthetic fixture.
//!
//! Two real, reproduced blockers shaped this crate's scope:
//!
//! 1. `flatpak-builder`'s sandboxed build step (a FUSE-backed rofiles
//!    overlay via bubblewrap) needs real elevated privileges. Neither
//!    `fuse3` installed nor a plain `--device /dev/fuse` are enough on
//!    their own — the mount itself still fails with "Operation not
//!    permitted." Dagger's `--insecure-root-capabilities` on the
//!    `with-exec` that runs `flatpak-builder` (its equivalent of `docker
//!    run --privileged`) is what actually makes it work — confirmed
//!    directly, and it's still routed entirely through Dagger (ADR-0001),
//!    nothing here spawns `docker`/`cross` directly.
//! 2. `paws ci --toolchain flatpak` runs `flatpak-builder --build-only`,
//!    not a full bundle export. The metadata "finish" phase runs
//!    `appstream-compose` inside its *own* inner bubblewrap sandbox — a
//!    binary Debian bookworm doesn't ship anymore (superseded by
//!    `appstreamcli compose`), and that inner sandbox doesn't see anything
//!    installed on the outer container's PATH, so a host-side wrapper
//!    script doesn't help either. `--build-only` stops right after
//!    compiling and installing into the Flatpak module tree, before that
//!    phase ever runs — which is what CI actually needs to verify (does
//!    the app build inside its own Flatpak sandbox), the same
//!    "build-verified, not a full package" scope `paws-release`'s macOS
//!    targets already use for a different reason.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The Flatpak builder Dockerfile, embedded at compile time from
/// `builders/flatpak/Dockerfile`. `paws ci` runs from inside whatever
/// *target* repo it's checking, not from inside `paws`'s own source tree —
/// a plain repo-relative `builders/flatpak` path would resolve against the
/// wrong directory the moment `paws` is run anywhere but its own repo, the
/// same bug class already fixed for `paws-tauri`.
const FLATPAK_DOCKERFILE: &str = include_str!("../../../builders/flatpak/Dockerfile");

/// Writes the embedded Flatpak builder Dockerfile to a temp directory and
/// returns that directory's path, suitable for `dagger_pipeline_args`'s
/// `builder_dir` argument.
pub fn write_builder_dockerfile() -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("paws-builders").join("flatpak");
    std::fs::create_dir_all(&dir)
        .context("failed to create temp dir for the flatpak builder Dockerfile")?;
    std::fs::write(dir.join("Dockerfile"), FLATPAK_DOCKERFILE)
        .context("failed to write the flatpak builder Dockerfile")?;
    Ok(dir)
}

/// Common locations a Flatpak manifest lives in a real repo, checked in
/// order. `packaging/flatpak/` matches this crate's own real-world
/// reference (oled-wallpaper); a bare `flatpak/` dir and the repo root are
/// the next most common conventions.
const MANIFEST_SEARCH_DIRS: &[&str] = &["packaging/flatpak", "flatpak", "."];

pub struct FlatpakProject {
    pub manifest_path: PathBuf,
    pub app_id: String,
}

/// Finds a Flatpak manifest (a `.yml`/`.yaml`/`.json` file with a top-level
/// `app-id:`/`id:` scalar) in `dir`, checking [`MANIFEST_SEARCH_DIRS`] in
/// order. Doesn't parse full YAML/JSON — flatpak manifests conventionally
/// put `app-id` as a plain top-level scalar, so a line scan is enough and
/// avoids pulling in a YAML parser for one field.
pub fn detect_project(dir: &Path) -> Result<FlatpakProject> {
    for search_dir in MANIFEST_SEARCH_DIRS {
        let candidate_dir = dir.join(search_dir);
        if !candidate_dir.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&candidate_dir) else {
            continue;
        };
        let mut file_names: Vec<std::ffi::OsString> =
            entries.flatten().map(|e| e.file_name()).collect();
        file_names.sort();
        for file_name in file_names {
            let path = candidate_dir.join(&file_name);
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                continue;
            };
            if !matches!(ext, "yml" | "yaml" | "json") {
                continue;
            }
            let Ok(contents) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(app_id) = extract_app_id(&contents) {
                // Relative to `dir`, not absolute — this ends up as an
                // argument to `flatpak-builder` running inside a container
                // where `dir`'s contents are mounted at `/src`, not at
                // `dir`'s original host path (a real bug caught by
                // verifying against oled-wallpaper end to end: an absolute
                // host path doesn't exist inside the container at all).
                return Ok(FlatpakProject {
                    manifest_path: Path::new(search_dir).join(&file_name),
                    app_id,
                });
            }
        }
    }
    anyhow::bail!(
        "no Flatpak manifest found in {} (checked {})",
        dir.display(),
        MANIFEST_SEARCH_DIRS.join(", ")
    );
}

pub fn is_flatpak_project(dir: &Path) -> bool {
    detect_project(dir).is_ok()
}

fn extract_app_id(manifest_contents: &str) -> Option<String> {
    for line in manifest_contents.lines() {
        let trimmed = line.trim();
        for key in ["app-id:", "\"app-id\":", "id:", "\"id\":"] {
            if let Some(rest) = trimmed.strip_prefix(key) {
                let value = rest.trim().trim_end_matches(',').trim_matches('"').trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`)
/// that builds the `builders/flatpak` image (`builder_dir`, materialized
/// from the binary — see the CLI's `write_builder_dockerfile`-style
/// helper) and runs `flatpak-builder --build-only --force-clean build-dir
/// <manifest>` against `project`'s manifest, relative to `source_dir`
/// mounted at `/src`, with `--insecure-root-capabilities` (see this
/// module's doc comment for why).
pub fn dagger_pipeline_args(
    project: &FlatpakProject,
    source_dir: &str,
    builder_dir: &str,
) -> Vec<String> {
    let created_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let build_args =
        format!("BUILDER_VERSION=dev,BUILDER_REVISION=unknown,BUILDER_CREATED={created_unix}");

    let manifest_relative = project.manifest_path.to_string_lossy();

    vec![
        "host".into(),
        "directory".into(),
        format!("--path={builder_dir}"),
        "docker-build".into(),
        format!("--build-args={build_args}"),
        "with-mounted-directory".into(),
        "--path=/src".into(),
        format!("--source={source_dir}"),
        "with-workdir".into(),
        "--path=/src".into(),
        "with-exec".into(),
        "--insecure-root-capabilities".into(),
        format!("--args=flatpak-builder,--build-only,--force-clean,build-dir,{manifest_relative}"),
        "stdout".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("paws-flatpak-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn extracts_app_id_from_yaml_manifest() {
        let manifest =
            "app-id: ninja.boop.OledWallpaper\nruntime: org.freedesktop.Platform//25.08\n";
        assert_eq!(
            extract_app_id(manifest),
            Some("ninja.boop.OledWallpaper".to_string())
        );
    }

    #[test]
    fn extracts_app_id_from_json_manifest() {
        let manifest = "{\n  \"app-id\": \"org.example.App\",\n  \"runtime\": \"org.freedesktop.Platform\"\n}\n";
        assert_eq!(
            extract_app_id(manifest),
            Some("org.example.App".to_string())
        );
    }

    #[test]
    fn falls_back_to_bare_id_key() {
        let manifest = "id: org.example.Legacy\nruntime: org.freedesktop.Platform\n";
        assert_eq!(
            extract_app_id(manifest),
            Some("org.example.Legacy".to_string())
        );
    }

    #[test]
    fn detects_manifest_under_packaging_flatpak() {
        let dir = temp_dir("packaging");
        let manifest_dir = dir.join("packaging/flatpak");
        fs::create_dir_all(&manifest_dir).unwrap();
        fs::write(
            manifest_dir.join("com.example.App.yml"),
            "app-id: com.example.App\n",
        )
        .unwrap();

        let project = detect_project(&dir).unwrap();
        assert_eq!(project.app_id, "com.example.App");
        assert_eq!(
            project.manifest_path,
            Path::new("packaging/flatpak").join("com.example.App.yml"),
            "manifest_path must be relative to dir, not absolute - it becomes an argument \
             inside a container where dir's contents are mounted at /src, not at dir's \
             original host path"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_when_no_manifest_found() {
        let dir = temp_dir("none");
        assert!(detect_project(&dir).is_err());
        assert!(!is_flatpak_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn flatpak_builder_dockerfile_exists() {
        let dockerfile = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("builders/flatpak")
            .join("Dockerfile");
        assert!(dockerfile.is_file(), "missing {dockerfile:?}");
    }

    #[test]
    fn write_builder_dockerfile_materializes_the_embedded_dockerfile() {
        let dir = write_builder_dockerfile().unwrap();
        let contents = fs::read_to_string(dir.join("Dockerfile")).unwrap();
        assert_eq!(contents, FLATPAK_DOCKERFILE);
    }

    #[test]
    fn pipeline_uses_insecure_root_capabilities_and_build_only() {
        let project = FlatpakProject {
            manifest_path: PathBuf::from("packaging/flatpak/com.example.App.yml"),
            app_id: "com.example.App".to_string(),
        };
        let args = dagger_pipeline_args(&project, "/host/src", "/tmp/some-builder-dir");
        assert_eq!(args[0], "host");
        assert_eq!(args[2], "--path=/tmp/some-builder-dir");
        assert!(args.contains(&"--insecure-root-capabilities".to_string()));
        assert!(
            args.iter()
                .any(|a| a.contains("flatpak-builder,--build-only")),
            "args: {args:?}"
        );
        assert!(
            args.iter()
                .any(|a| a.contains("packaging/flatpak/com.example.App.yml")),
            "args: {args:?}"
        );
        assert_eq!(args.last(), Some(&"stdout".to_string()));
    }
}
