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
//! way any other dependency is.
//!
//! Reuses `paws-java`'s `builders/java` image (JDK 21 + JDK 25 side by
//! side — see `crates/paws-java`'s module doc for the full finding on why
//! one pin can't cover both an old Gradle <=8.10 project and a real
//! `java.toolchain.languageVersion = JavaLanguageVersion.of(25)`
//! declaration) rather than a plain image pull, embedding the same
//! Dockerfile independently (matching how every other per-toolchain crate
//! in this workspace — `paws-go`, `paws-rust`, `paws-python` —
//! independently owns its own pipeline builder rather than reaching into a
//! sibling crate for one, even when the underlying image is identical).
//!
//! This is also what closes `docs/ROADMAP.md`'s `Java + Kotlin` row for
//! free, the same "zero code changes" story as `Go + C/C++ (cgo)`: a
//! single Gradle module with both `.java` and `.kt` sources compiles
//! through this exact pipeline unchanged, because Gradle's `java` and
//! `kotlin` plugins already handle mixed compilation themselves —
//! `examples/java-kotlin-mixed-fixture` exists purely to prove that.
//! Requires the project's own `gradlew` wrapper, same reasoning as
//! `paws-java`.

use paws_core::Pipeline;
use std::path::{Path, PathBuf};

use anyhow::Result;

/// The `builders/java` Dockerfile, embedded independently from
/// `paws-java`'s own copy — see this module's doc comment on why that
/// duplication is deliberate.
const JAVA_DOCKERFILE: &str = include_str!("../../../builders/java/Dockerfile");

/// Writes the embedded `builders/java` Dockerfile to a temp directory and
/// returns that directory's path, suitable for `dagger_pipeline_args`'s
/// `builder_dir` argument. Dagger's own `BuildKit` layer caching means this
/// resolves to the same cached image `paws-java` already built in the same
/// `paws ci` process/host, not a second independent build.
pub fn write_builder_dockerfile() -> Result<PathBuf> {
    paws_core::write_builder_dockerfile("java", JAVA_DOCKERFILE)
}

/// Every `.kt`/`.kts` file under `dir`, recursing but skipping hidden
/// directories — same walking strategy as `paws_go::go_files`.
fn kotlin_files(dir: &Path) -> Vec<PathBuf> {
    paws_core::find_files_with_extension(dir, &["kt", "kts"])
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
/// for `source_dir`: builds `builder_dir` (see [`write_builder_dockerfile`]),
/// then runs `sh gradlew build` — identical in shape to
/// `paws_java::dagger_pipeline_args`'s Gradle path.
pub fn dagger_pipeline_args(source_dir: &str, builder_dir: &str) -> Vec<String> {
    Pipeline::from_builder_image(builder_dir)
        .mount("/src", source_dir)
        .workdir("/src")
        .exec(["sh", "gradlew", "build"])
        .stdout()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn temp_dir(name: &str) -> PathBuf {
        paws_core::test_support::scratch_dir("kotlin", name)
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
    fn write_builder_dockerfile_materializes_the_embedded_dockerfile() {
        let dir = write_builder_dockerfile().unwrap();
        let contents = fs::read_to_string(dir.join("Dockerfile")).unwrap();
        assert_eq!(contents, JAVA_DOCKERFILE);
    }

    #[test]
    fn pipeline_builds_the_builder_then_runs_the_wrapper_via_sh() {
        let args = dagger_pipeline_args("/host/src", "/builder/dir");
        assert_eq!(args[0], "host");
        assert_eq!(args[1], "directory");
        assert_eq!(args[2], "--path=/builder/dir");
        assert_eq!(args[3], "docker-build");
        assert!(args.contains(&"--args=sh,gradlew,build".to_string()));
        assert_eq!(args.last(), Some(&"stdout".to_string()));
    }
}
