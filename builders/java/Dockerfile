# syntax=docker/dockerfile:1.7
#
# Builder for `paws ci --toolchain java`/`kotlin`: JDK 21 + JDK 25 side by
# side under /usr/lib/jvm/ -- the location Gradle's own toolchain
# auto-detection scans on Linux, confirmed for real (no
# `org.gradle.java.installations.paths` configuration needed in the target
# project). JAVA_HOME/PATH default to 21 so Gradle <=8.10 keeps working --
# confirmed for real that it can't even *launch* on a JDK-25 host JVM at
# all, an "Unsupported class file major version 69" failure in Gradle's own
# Groovy DSL parsing, unrelated to whether the target project uses Kotlin
# -- while a project whose own build declares
# `java.toolchain.languageVersion = JavaLanguageVersion.of(25)` (real,
# confirmed against mbround18/hytale-modding-template's plugin/build.gradle)
# resolves the 25 install sitting right alongside it, no toolchain
# auto-download needed. Built through Dagger, not `docker build` directly
# -- see builders/README.md.
#
# One combined image rather than swapping crates/paws-java's/
# crates/paws-kotlin's plain `eclipse-temurin:21-jdk-jammy` pull for this,
# since neither JDK 21 alone (breaks a real toolchain=25 requirement) nor
# JDK 25 alone (breaks any real Gradle <=8.10 project) covers both cases a
# single public image tag can express -- exactly the "needs multiple
# toolchains combined" reason every other `builders/*` image exists (see
# docs/ROADMAP.md's "How a new stack gets added").

FROM eclipse-temurin:25-jdk-jammy AS jdk25

FROM eclipse-temurin:21-jdk-jammy

COPY --from=jdk25 /opt/java/openjdk /usr/lib/jvm/temurin-25-jdk-amd64
RUN ln -s /opt/java/openjdk /usr/lib/jvm/temurin-21-jdk-amd64

ARG BUILDER_VERSION=0.0.0
ARG BUILDER_REVISION=unknown
ARG BUILDER_CREATED=unknown

LABEL org.opencontainers.image.title="paws-builder-java" \
      org.opencontainers.image.description="JDK 21 + JDK 25 side by side for paws ci --toolchain java/kotlin" \
      org.opencontainers.image.source="https://github.com/mbround18/paws" \
      org.opencontainers.image.version="${BUILDER_VERSION}" \
      org.opencontainers.image.revision="${BUILDER_REVISION}" \
      org.opencontainers.image.created="${BUILDER_CREATED}" \
      io.paws.targets="java,kotlin"
