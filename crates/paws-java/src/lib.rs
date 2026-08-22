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

use std::path::Path;

/// `eclipse-temurin:21-jdk-jammy` — gh-reusable's own `DEFAULT_TEMURIN_IMAGE`
/// (`eclipse-temurin:<version>-jdk-trixie`) does not actually exist on
/// Docker Hub (confirmed directly: no Debian-suffixed tag is published for
/// `eclipse-temurin`, only Ubuntu/Alpine/Windows variants), so this isn't a
/// port of that constant — `-jammy` is the real, verified equivalent for
/// the same JDK 21 Temurin distribution gh-reusable defaults to.
pub const BASE_IMAGE: &str = "eclipse-temurin:21-jdk-jammy";

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
pub fn detect_project(dir: &Path) -> anyhow::Result<BuildSystem> {
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
/// for `source_dir`: `sh mvnw -B verify` (Maven) or `sh gradlew build`
/// (Gradle), against [`BASE_IMAGE`].
pub fn dagger_pipeline_args(build_system: BuildSystem, source_dir: &str) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "container".into(),
        "from".into(),
        format!("--address={BASE_IMAGE}"),
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
    fn maven_pipeline_runs_the_wrapper_via_sh_in_batch_mode() {
        let args = dagger_pipeline_args(BuildSystem::Maven, "/host/src");
        assert_eq!(
            args,
            vec![
                "container".to_string(),
                "from".to_string(),
                "--address=eclipse-temurin:21-jdk-jammy".to_string(),
                "with-mounted-directory".to_string(),
                "--path=/src".to_string(),
                "--source=/host/src".to_string(),
                "with-workdir".to_string(),
                "--path=/src".to_string(),
                "with-exec".to_string(),
                "--args=sh,mvnw,-B,verify".to_string(),
                "stdout".to_string(),
            ]
        );
    }

    #[test]
    fn gradle_pipeline_runs_the_wrapper_via_sh() {
        let args = dagger_pipeline_args(BuildSystem::Gradle, "/host/src");
        assert!(args.contains(&"--args=sh,gradlew,build".to_string()));
    }
}
