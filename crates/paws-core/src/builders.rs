//! Materializing an embedded `builders/*` Dockerfile onto the host.
//!
//! Several toolchains build against a purpose-built image rather than a
//! stock one. The Dockerfile is embedded at compile time (`include_str!`) so
//! it is readable when `paws` runs inside a consumer repo rather than its own
//! checkout, then written to a temp directory whose path becomes
//! `dagger core`'s build context.
//!
//! Seven crates wrote that out independently — three of them had already
//! factored an identical private `write_dockerfile(name, contents)` and then
//! that was copied too. This is that function, once.

use std::path::PathBuf;

use anyhow::{Context, Result};

/// Writes `contents` as a `Dockerfile` under a per-builder temp directory and
/// returns that directory, ready to hand to
/// [`crate::Pipeline::from_builder_image`].
///
/// `name` identifies the builder (`"rust"`, `"java"`, `"tauri-linux"`, …) and
/// is used both for the directory and in any error. Writing on every call is
/// deliberate and cheap: it keeps a stale Dockerfile from a previous `paws`
/// version from being reused, and Dagger's own `BuildKit` layer caching means
/// an unchanged Dockerfile still resolves to the cached image rather than a
/// fresh build.
pub fn write_builder_dockerfile(name: &str, contents: &str) -> Result<PathBuf> {
    let dir = builder_dir(name);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create temp dir for the {name} builder Dockerfile"))?;
    std::fs::write(dir.join("Dockerfile"), contents)
        .with_context(|| format!("failed to write the {name} builder Dockerfile"))?;
    Ok(dir)
}

/// Where [`write_builder_dockerfile`] puts `name`'s build context.
pub fn builder_dir(name: &str) -> PathBuf {
    std::env::temp_dir().join("paws-builders").join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_the_dockerfile_and_returns_its_directory() {
        let name = format!("core-test-{}", std::process::id());
        let dir = write_builder_dockerfile(&name, "FROM scratch\n").unwrap();

        assert_eq!(dir, builder_dir(&name));
        assert_eq!(
            std::fs::read_to_string(dir.join("Dockerfile")).unwrap(),
            "FROM scratch\n"
        );

        // Rewriting replaces the previous contents rather than appending or
        // failing on the existing directory.
        let dir = write_builder_dockerfile(&name, "FROM alpine\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("Dockerfile")).unwrap(),
            "FROM alpine\n"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn each_builder_gets_its_own_directory() {
        assert_ne!(builder_dir("rust"), builder_dir("java"));
        assert!(builder_dir("tauri-linux").ends_with("paws-builders/tauri-linux"));
    }
}
