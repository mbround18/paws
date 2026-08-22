//! Native Java CI support for `paws ci --toolchain java`. Like `paws-go`,
//! there's no `gh-reusable` build/test function to port for parity —
//! `gh-reusable` only ever had `setupJava` (a container setup step picking
//! a JDK distribution/image, no build/test steps of its own; see
//! `packages/dagger-module/src/index.ts`). This crate is a new, native
//! implementation.
//!
//! Deliberately **requires** the project's own Maven/Gradle wrapper
//! (`mvnw`/`gradlew`) rather than falling back to a system-installed
//! `mvn`/`gradle` this crate would have to pick a version for and install
//! itself — the wrapper already pins an exact, reproducible build-tool
//! version (the same reason most real Java repos commit one), so this
//! crate doesn't need to make that decision or reimplement an installer.
//! A repo without a wrapper committed gets a clear error rather than a
//! silent, unpinned "whatever `apt` happens to give you" build.
//!
//! Invoked as `sh mvnw`/`sh gradlew` rather than `./mvnw`/`./gradlew`
//! directly, so a missing execute bit on the wrapper script (e.g. cloned
//! on a filesystem/config where it wasn't preserved) doesn't fail the
//! build over a permissions technicality the script's own content doesn't
//! care about.
//!
//! Builds through `builders/java` (JDK 21 + JDK 25 side by side), not a
//! plain public-image pull — confirmed for real this needed *both*
//! versions available, not a single pin: Gradle <=8.10 can't even launch
//! on a JDK-25 host JVM at all (`Unsupported class file major version 69`,
//! Gradle's own Groovy DSL parsing, unrelated to whether the target
//! project uses Kotlin), while a real, modern project (confirmed against
//! `mbround18/hytale-modding-template`, pinned to Gradle 9.3.1) can
//! legitimately declare `java.toolchain.languageVersion =
//! JavaLanguageVersion.of(25)` and needs a real JDK 25 install to resolve
//! it against, without relying on network-based toolchain auto-provisioning
//! the target project may not have configured. Both installs sit under
//! `/usr/lib/jvm/` — the location Gradle's own toolchain auto-detection
//! scans on Linux, confirmed for real (no `gradle.properties`/
//! `org.gradle.java.installations.paths` changes needed in the target
//! project) — with `JAVA_HOME`/`PATH` defaulting to 21, so old and new
//! Gradle pins both just work through the same image.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// The `builders/java` Dockerfile (JDK 21 + JDK 25 side by side; see this
/// module's doc comment), embedded at compile time — `paws ci` runs from
/// inside whatever *target* repo it's checking, not from inside `paws`'s
/// own source tree, so a repo-relative `builders/java` path would resolve
/// against the wrong directory; materializing the embedded contents to a
/// temp dir (see [`write_builder_dockerfile`]) makes this correct
/// regardless of where `paws` is invoked from — same reasoning
/// `paws-tauri`/`paws-flatpak` already established for their own builders.
const JAVA_DOCKERFILE: &str = include_str!("../../../builders/java/Dockerfile");

fn write_dockerfile(name: &str, contents: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join("paws-builders").join(name);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create temp dir for the {name} builder Dockerfile"))?;
    std::fs::write(dir.join("Dockerfile"), contents)
        .with_context(|| format!("failed to write the {name} builder Dockerfile"))?;
    Ok(dir)
}

/// Writes the embedded `builders/java` Dockerfile to a temp directory and
/// returns that directory's path, suitable for `dagger_pipeline_args`'s
/// `builder_dir` argument.
pub fn write_builder_dockerfile() -> Result<PathBuf> {
    write_dockerfile("java", JAVA_DOCKERFILE)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildSystem {
    Maven,
    Gradle,
}

impl BuildSystem {
    pub fn as_str(&self) -> &'static str {
        match self {
            BuildSystem::Maven => "maven",
            BuildSystem::Gradle => "gradle",
        }
    }

    fn wrapper(&self) -> &'static str {
        match self {
            BuildSystem::Maven => "mvnw",
            BuildSystem::Gradle => "gradlew",
        }
    }

    fn build_command(&self) -> &'static [&'static str] {
        match self {
            BuildSystem::Maven => &["-B", "verify"],
            BuildSystem::Gradle => &["build"],
        }
    }
}

/// A Java project has a Maven (`pom.xml`) or Gradle (`build.gradle`/
/// `build.gradle.kts`) build file at its root.
pub fn is_java_project(dir: &Path) -> bool {
    dir.join("pom.xml").is_file()
        || dir.join("build.gradle").is_file()
        || dir.join("build.gradle.kts").is_file()
}

/// Detects the build system and confirms its wrapper script is committed
/// (see this module's doc comment on why the wrapper is required rather
/// than a system `mvn`/`gradle` fallback). Maven is checked first, so a
/// repo with both a stray `build.gradle` and a real `pom.xml` is treated
/// as Maven — an unlikely combination in practice, not a meaningful
/// ambiguity to design around further.
pub fn detect_project(dir: &Path) -> Result<BuildSystem> {
    let build_system = if dir.join("pom.xml").is_file() {
        BuildSystem::Maven
    } else if dir.join("build.gradle").is_file() || dir.join("build.gradle.kts").is_file() {
        BuildSystem::Gradle
    } else {
        anyhow::bail!(
            "no pom.xml or build.gradle(.kts) found in {}",
            dir.display()
        );
    };

    if !dir.join(build_system.wrapper()).is_file() {
        anyhow::bail!(
            "{} project detected in {}, but no {} wrapper script is committed — paws ci --toolchain java requires one (see crates/paws-java's module doc for why)",
            build_system.as_str(),
            dir.display(),
            build_system.wrapper()
        );
    }

    Ok(build_system)
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`)
/// for `source_dir`: builds `builder_dir` (see [`write_builder_dockerfile`]
/// — Dagger's own BuildKit layer caching means the slow JDK-25 `COPY` only
/// actually runs once per unchanged Dockerfile, not on every `paws ci`
/// invocation), then runs `sh mvnw -B verify` (Maven) or `sh gradlew build`
/// (Gradle).
pub fn dagger_pipeline_args(
    build_system: BuildSystem,
    source_dir: &str,
    builder_dir: &str,
) -> Vec<String> {
    let created_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let build_args =
        format!("BUILDER_VERSION=dev,BUILDER_REVISION=unknown,BUILDER_CREATED={created_unix}");

    let mut args: Vec<String> = vec![
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
    ];

    let mut command_args = vec!["sh".to_string(), build_system.wrapper().to_string()];
    command_args.extend(build_system.build_command().iter().map(|s| s.to_string()));

    args.push("with-exec".into());
    args.push(format!("--args={}", command_args.join(",")));
    args.push("stdout".into());
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("paws-java-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detects_maven_project_from_pom_xml() {
        let dir = temp_dir("maven-detect");
        assert!(!is_java_project(&dir));
        fs::write(dir.join("pom.xml"), "<project></project>").unwrap();
        assert!(is_java_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_gradle_project_from_build_gradle_kts() {
        let dir = temp_dir("gradle-detect");
        assert!(!is_java_project(&dir));
        fs::write(dir.join("build.gradle.kts"), "").unwrap();
        assert!(is_java_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_when_no_build_file_present() {
        let dir = temp_dir("no-build-file");
        assert!(detect_project(&dir).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_when_pom_xml_exists_but_mvnw_wrapper_is_missing() {
        let dir = temp_dir("maven-no-wrapper");
        fs::write(dir.join("pom.xml"), "<project></project>").unwrap();
        let err = detect_project(&dir).unwrap_err().to_string();
        assert!(
            err.contains("mvnw"),
            "error should name the missing wrapper: {err}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_maven_when_both_wrapper_and_pom_present() {
        let dir = temp_dir("maven-with-wrapper");
        fs::write(dir.join("pom.xml"), "<project></project>").unwrap();
        fs::write(dir.join("mvnw"), "#!/bin/sh\n").unwrap();
        assert_eq!(detect_project(&dir).unwrap(), BuildSystem::Maven);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_gradle_when_both_wrapper_and_build_file_present() {
        let dir = temp_dir("gradle-with-wrapper");
        fs::write(dir.join("build.gradle"), "").unwrap();
        fs::write(dir.join("gradlew"), "#!/bin/sh\n").unwrap();
        assert_eq!(detect_project(&dir).unwrap(), BuildSystem::Gradle);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn write_builder_dockerfile_materializes_the_embedded_dockerfile() {
        let dir = write_builder_dockerfile().unwrap();
        let contents = fs::read_to_string(dir.join("Dockerfile")).unwrap();
        assert_eq!(contents, JAVA_DOCKERFILE);
    }

    #[test]
    fn maven_pipeline_builds_the_builder_then_runs_the_wrapper_in_batch_mode() {
        let args = dagger_pipeline_args(BuildSystem::Maven, "/host/src", "/builder/dir");
        assert_eq!(args[0], "host");
        assert_eq!(args[1], "directory");
        assert_eq!(args[2], "--path=/builder/dir");
        assert_eq!(args[3], "docker-build");
        assert!(args.contains(&"--args=sh,mvnw,-B,verify".to_string()));
        assert_eq!(args.last(), Some(&"stdout".to_string()));
    }

    #[test]
    fn gradle_pipeline_runs_the_wrapper_via_sh() {
        let args = dagger_pipeline_args(BuildSystem::Gradle, "/host/src", "/builder/dir");
        assert!(args.contains(&"--args=sh,gradlew,build".to_string()));
    }
}
