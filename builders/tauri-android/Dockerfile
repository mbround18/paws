# syntax=docker/dockerfile:1.7
#
# Builder for Tauri apps on Android. JDK 17 + Android SDK (platform-tools,
# a platform, build-tools, the NDK) + Rust's Android cross targets + Node
# (for the frontend build) — see
# https://tauri.app/start/prerequisites/#android, which is explicit that
# only ANDROID_HOME and NDK_HOME need to be set (no specific SDK/NDK
# version pinned upstream); the versions below are just this image's
# current pin, bumpable via the ARGs.
#
# Assumes the target repo has already run `tauri android init` (i.e.
# `src-tauri/gen/android` is committed) — paws builds what's already in the
# repo, it doesn't scaffold mobile projects itself.
#
# Unlike `builders/macos` (osxcross lets Rust cross-compile against the
# Darwin ABI using Apple's redistributed SDK headers), there is no
# equivalent for iOS: Tauri's iOS build shells out to Xcode's own
# `xcodebuild`, which only runs on genuine macOS under Apple's license
# terms. Android has no such constraint — the SDK/NDK are plain
# redistributable downloads and the whole toolchain runs on Linux, which is
# what makes this image possible at all.

FROM rust:1-bookworm

ARG BUILDER_VERSION=0.0.0
ARG BUILDER_REVISION=unknown
ARG BUILDER_CREATED=unknown
ARG NODE_MAJOR=22
ARG ANDROID_CMDLINE_TOOLS_VERSION=9862592
ARG ANDROID_CMDLINE_TOOLS_SHA1=a787f0c35d3970f2ad634f7d91a2e7bc49e2c7f5
ARG ANDROID_PLATFORM=android-34
ARG ANDROID_BUILD_TOOLS_VERSION=34.0.0
ARG ANDROID_NDK_VERSION=27.0.12077973

LABEL org.opencontainers.image.title="paws-builder-tauri-android" \
      org.opencontainers.image.description="JDK + Android SDK/NDK + Rust + Node builder for Tauri apps on Android" \
      org.opencontainers.image.source="https://github.com/mbround18/paws" \
      org.opencontainers.image.version="${BUILDER_VERSION}" \
      org.opencontainers.image.revision="${BUILDER_REVISION}" \
      org.opencontainers.image.created="${BUILDER_CREATED}" \
      io.paws.targets="tauri-android"

ENV ANDROID_HOME=/opt/android-sdk
ENV ANDROID_SDK_ROOT=${ANDROID_HOME}
ENV NDK_HOME=${ANDROID_HOME}/ndk/${ANDROID_NDK_VERSION}
ENV PATH="${ANDROID_HOME}/cmdline-tools/latest/bin:${ANDROID_HOME}/platform-tools:${PATH}"

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update -qq && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        curl ca-certificates unzip openjdk-17-jdk-headless && \
    ln -sf "/usr/lib/jvm/java-17-openjdk-$(dpkg --print-architecture)" /usr/lib/jvm/default-java && \
    curl -fsSL "https://deb.nodesource.com/setup_${NODE_MAJOR}.x" | bash - && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends nodejs

ENV JAVA_HOME=/usr/lib/jvm/default-java

RUN mkdir -p "${ANDROID_HOME}/cmdline-tools" && \
    curl -fsSL -o /tmp/cmdline-tools.zip \
        "https://dl.google.com/android/repository/commandlinetools-linux-${ANDROID_CMDLINE_TOOLS_VERSION}_latest.zip" && \
    echo "${ANDROID_CMDLINE_TOOLS_SHA1}  /tmp/cmdline-tools.zip" | sha1sum -c - && \
    unzip -q /tmp/cmdline-tools.zip -d "${ANDROID_HOME}/cmdline-tools" && \
    mv "${ANDROID_HOME}/cmdline-tools/cmdline-tools" "${ANDROID_HOME}/cmdline-tools/latest" && \
    rm /tmp/cmdline-tools.zip && \
    yes | sdkmanager --licenses --sdk_root="${ANDROID_HOME}" >/dev/null && \
    sdkmanager --sdk_root="${ANDROID_HOME}" \
        "platform-tools" \
        "platforms;${ANDROID_PLATFORM}" \
        "build-tools;${ANDROID_BUILD_TOOLS_VERSION}" \
        "ndk;${ANDROID_NDK_VERSION}"

RUN rustup target add \
    aarch64-linux-android \
    armv7-linux-androideabi \
    i686-linux-android \
    x86_64-linux-android

WORKDIR /src
