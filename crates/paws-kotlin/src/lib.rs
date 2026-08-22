//! Native Kotlin (JVM) CI support for `paws ci --toolchain kotlin`. Like
//! `paws-go`/`paws-java`, there's no `gh-reusable` Kotlin build/test
//! function to port — `gh-reusable` never had a Kotlin-specific setup step
//! at all (Java's is the closest, and even that was container setup only;
//! see `crates/paws-java`'s module doc).
//!
//! Gradle-only, deliberately: real Kotlin projects are overwhelmingly
//! Gradle-based (the `kotlin-maven-plugin` exists but is rare in practice),
//! and unlike Java, Kotlin's compiler isn't part of the JDK at all — it's
//! entirely a Gradle plugin dependency, fetched by Gradle itself the same
//! way any other dependency is. That means **no new base image or build
//! commands are needed here**: this crate reuses the exact same
//! `eclipse-temurin:21-jdk-jammy` + `sh gradlew build` pipeline shape as
//! `paws-java`'s Gradle path (duplicated rather than shared, matching how
//! every other per-toolchain crate in this workspace — `paws-go`,
//! `paws-rust`, `paws-python` — independently owns its own pipeline
//! builder rather than reaching into a sibling crate for one).
//!
//! This is also what closes `docs/ROADMAP.md`'s `Java + Kotlin` row for
//! free, the same "zero code changes" story as `Go + C/C++ (cgo)`: a
//! single Gradle module with both `.java` and `.kt` sources compiles
//! through this exact pipeline unchanged, because Gradle's `java` and
//! `kotlin` plugins already handle mixed compilation themselves —
//! `examples/java-kotlin-mixed-fixture` exists purely to prove that.
//! Requires the project's own `gradlew` wrapper, same reasoning as
//! `paws-java`.

use std::path::{Path, PathBuf};

use anyhow::Result;

/// Same image `paws-java` uses — Kotlin needs nothing beyond a working JDK
/// plus network access for Gradle to fetch the Kotlin compiler itself.
/// Pinned to JDK 21, not the newer JDK 25 LTS, for a reason specific to
/// *this* crate: confirmed for real that a JDK-25 build JVM breaks a real
/// `gradlew build` with Gradle 8.10 + Kotlin Gradle plugin 1.9.24 (still
/// common in the wild), even though `gradle --version` alone succeeds on
/// it — see `paws_java::BASE_IMAGE`'s doc comment for the full finding.
pub const BASE_IMAGE: &str = "eclipse-temurin:21-jdk-jammy";

/// Every `.kt`/`.kts` file under `dir`, recursing but skipping hidden
/// directories — same walking strategy as `paws_go::go_files`.
fn kotlin_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') {
                continue;
            }
            files.extend(kotlin_files(&path));
        } else {
            match path.extension().and_then(|e| e.to_str()) {
                Some("kt") | Some("kts") => files.push(path),
                _ => {}
            }
        }
    }
    files
}

/// A Kotlin project has real `.kt` source files (the purpose-built
/// signal — a build file merely *mentioning* Kotlin isn't the same as
/// actually having Kotlin code) under a Gradle build (`build.gradle`/
/// `build.gradle.kts` present, since that's the only build system this
/// crate drives).
pub fn is_kotlin_project(dir: &Path) -> bool {
    if !dir.join("build.gradle").is_file() && !dir.join("build.gradle.kts").is_file() {
        return false;
    }
    !kotlin_files(dir).is_empty()
}

/// Confirms `dir` is a Kotlin project (see [`is_kotlin_project`]) and that
/// its `gradlew` wrapper is committed — required for the same reason
/// `paws-java` requires one: an exact, reproducible build-tool version
/// this crate doesn't have to pick or install itself.
pub fn detect_project(dir: &Path) -> Result<()> {
    if !is_kotlin_project(dir) {
        anyhow::bail!(
            "no Kotlin source (.kt files under a Gradle build) found in {}",
            dir.display()
        );
    }
    if !dir.join("gradlew").is_file() {
        anyhow::bail!(
            "kotlin project detected in {}, but no gradlew wrapper script is committed — paws ci --toolchain kotlin requires one (see crates/paws-kotlin's module doc for why)",
            dir.display()
        );
    }
    Ok(())
}

/// Builds the `dagger core <chain>` argument list (see `paws_dagger::core`)
/// for `source_dir`: `sh gradlew build` against [`BASE_IMAGE`] — identical
/// in shape to `paws_java::dagger_pipeline_args`'s Gradle path.
pub fn dagger_pipeline_args(source_dir: &str) -> Vec<String> {
    vec![
        "container".into(),
        "from".into(),
        format!("--address={BASE_IMAGE}"),
        "with-mounted-directory".into(),
        "--path=/src".into(),
        format!("--source={source_dir}"),
        "with-workdir".into(),
        "--path=/src".into(),
        "with-exec".into(),
        "--args=sh,gradlew,build".into(),
        "stdout".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("paws-kotlin-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn does_not_detect_a_gradle_project_with_no_kotlin_sources() {
        let dir = temp_dir("no-kt");
        fs::write(dir.join("build.gradle"), "").unwrap();
        assert!(!is_kotlin_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn does_not_detect_kotlin_files_without_a_gradle_build() {
        let dir = temp_dir("kt-no-gradle");
        fs::create_dir_all(dir.join("src/main/kotlin")).unwrap();
        fs::write(dir.join("src/main/kotlin/Main.kt"), "fun main() {}").unwrap();
        assert!(!is_kotlin_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detects_kotlin_project_from_kt_sources_plus_gradle_build() {
        let dir = temp_dir("kt-detect");
        fs::write(dir.join("build.gradle.kts"), "").unwrap();
        fs::create_dir_all(dir.join("src/main/kotlin")).unwrap();
        fs::write(dir.join("src/main/kotlin/Main.kt"), "fun main() {}").unwrap();
        assert!(is_kotlin_project(&dir));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn errors_when_kotlin_project_has_no_gradlew_wrapper() {
        let dir = temp_dir("kt-no-wrapper");
        fs::write(dir.join("build.gradle.kts"), "").unwrap();
        fs::create_dir_all(dir.join("src/main/kotlin")).unwrap();
        fs::write(dir.join("src/main/kotlin/Main.kt"), "fun main() {}").unwrap();
        let err = detect_project(&dir).unwrap_err().to_string();
        assert!(
            err.contains("gradlew"),
            "error should name the missing wrapper: {err}"
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn detect_project_succeeds_with_kotlin_sources_and_wrapper() {
        let dir = temp_dir("kt-with-wrapper");
        fs::write(dir.join("build.gradle.kts"), "").unwrap();
        fs::create_dir_all(dir.join("src/main/kotlin")).unwrap();
        fs::write(dir.join("src/main/kotlin/Main.kt"), "fun main() {}").unwrap();
        fs::write(dir.join("gradlew"), "#!/bin/sh\n").unwrap();
        assert!(detect_project(&dir).is_ok());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pipeline_runs_the_wrapper_via_sh() {
        let args = dagger_pipeline_args("/host/src");
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
                "--args=sh,gradlew,build".to_string(),
                "stdout".to_string(),
            ]
        );
    }
}
