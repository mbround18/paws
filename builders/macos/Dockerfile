# syntax=docker/dockerfile:1.7
#
# Builder for x86_64-apple-darwin and aarch64-apple-darwin, via osxcross
# (https://github.com/tpoechtrager/osxcross). SDK acquisition and Cargo
# linker/rustflags configuration follow the same approach validated in
# https://github.com/mbround18/setup-osxcross/blob/main/action.yml (a
# GitHub Action doing the equivalent setup) — the SDK comes from
# joseluisq/macosx-sdks' GitHub releases (a long-standing, widely-used
# community mirror), verified against its published sha256sum, rather than
# needing a user-supplied SDK tarball.
#
# Apple's SDK license does not permit Apple-distributed redistribution; this
# mirrors the same practice already in wide use across the Rust
# cross-compilation ecosystem (osxcross's own docs point at these community
# mirrors) rather than inventing a new distribution channel.

FROM debian:bookworm-slim AS osxcross-builder

# Pin the osxcross commit for reproducibility; bump deliberately, not on trust.
ARG OSXCROSS_COMMIT=27d21e4977c9751d01199c7a226a6faf494c3dd9
# Must be a tag at https://github.com/joseluisq/macosx-sdks/releases (11.0+
# for aarch64-apple-darwin support).
ARG MACOS_SDK_VERSION=14.0

# `docker-clean` (in every Debian/Ubuntu base image) deletes the .deb archives
# after each apt run, which would leave the cache mount below permanently
# empty -- see builders/README.md for the measurement.
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean && \
    echo 'Binary::apt::APT::Keep-Downloaded-Packages "true";' \
        > /etc/apt/apt.conf.d/keep-cache && \
    apt-get update -qq && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        ca-certificates clang cmake git patch libxml2-dev libssl-dev \
        lzma-dev libbz2-dev zlib1g-dev cpio automake libtool bash make xz-utils wget \
        python3

RUN git clone https://github.com/tpoechtrager/osxcross /opt/osxcross && \
    cd /opt/osxcross && git checkout "${OSXCROSS_COMMIT}"

# Fetch the SDK and verify it against the release's published checksum
# before trusting it — fail loudly on a mismatch, not silently continue.
RUN --mount=type=cache,target=/root/.cache/sdk-downloads \
    SDK_FILE="MacOSX${MACOS_SDK_VERSION}.sdk.tar.xz" && \
    SDK_URL="https://github.com/joseluisq/macosx-sdks/releases/download/${MACOS_SDK_VERSION}/${SDK_FILE}" && \
    CHECKSUM_URL="https://github.com/joseluisq/macosx-sdks/releases/download/${MACOS_SDK_VERSION}/sha256sum.txt" && \
    cd /root/.cache/sdk-downloads && \
    wget -nc "${SDK_URL}" && \
    wget -O expected-sha256sum.txt "${CHECKSUM_URL}" && \
    grep "${SDK_FILE}" expected-sha256sum.txt | sha256sum -c - && \
    cp "${SDK_FILE}" /opt/osxcross/tarballs/

RUN cd /opt/osxcross && UNATTENDED=1 ./build.sh

FROM rust:1-bookworm

ARG MACOS_SDK_VERSION=14.0

COPY --from=osxcross-builder /opt/osxcross/target /opt/osxcross/target

ENV PATH="/opt/osxcross/target/bin:${PATH}"

# osxcross's compiler wrappers shell out to a real `clang` — needed here too,
# not just in the osxcross-builder stage that built the wrappers themselves.
# `docker-clean` (in every Debian/Ubuntu base image) deletes the .deb archives
# after each apt run, which would leave the cache mount below permanently
# empty -- see builders/README.md for the measurement.
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean && \
    echo 'Binary::apt::APT::Keep-Downloaded-Packages "true";' \
        > /etc/apt/apt.conf.d/keep-cache && \
    apt-get update -qq && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends clang

RUN rustup target add x86_64-apple-darwin aarch64-apple-darwin

# osxcross names its compiler wrappers after the SDK's darwin version (e.g.
# "x86_64-apple-darwin23-clang"), which varies by SDK — Cargo's
# CARGO_TARGET_*_LINKER env var (and cc-rs's CC_*/AR_* env vars, needed by any
# dependency with a C component, e.g. `ring`) can't glob, so symlink to fixed
# names here (resolved once, at image-build time, from whatever SDK was
# actually used). Skips a target's symlinks (rather than failing the whole
# build) if the SDK didn't produce a toolchain for it — matches
# setup-osxcross's own handling of SDKs that predate aarch64-apple-darwin
# support.
RUN for triple in x86_64-apple-darwin aarch64-apple-darwin; do \
        for tool in clang clang++ ar ld; do \
            found="$(ls /opt/osxcross/target/bin/${triple}*-${tool} 2>/dev/null | head -1)"; \
            if [ -n "$found" ]; then \
                ln -sf "$found" "/opt/osxcross/target/bin/${triple}-${tool}"; \
            else \
                echo "warning: no ${tool} found for ${triple} in this SDK, skipping" >&2; \
            fi; \
        done; \
    done

ENV CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER=x86_64-apple-darwin-clang \
    CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=aarch64-apple-darwin-clang \
    CARGO_TARGET_X86_64_APPLE_DARWIN_RUSTFLAGS="-C link-arg=-undefined -C link-arg=dynamic_lookup -C link-arg=-fuse-ld=/opt/osxcross/target/bin/x86_64-apple-darwin-ld" \
    CARGO_TARGET_AARCH64_APPLE_DARWIN_RUSTFLAGS="-C link-arg=-undefined -C link-arg=dynamic_lookup -C link-arg=-fuse-ld=/opt/osxcross/target/bin/aarch64-apple-darwin-ld" \
    CC_x86_64_apple_darwin=x86_64-apple-darwin-clang \
    CC_aarch64_apple_darwin=aarch64-apple-darwin-clang \
    CXX_x86_64_apple_darwin=x86_64-apple-darwin-clang++ \
    CXX_aarch64_apple_darwin=aarch64-apple-darwin-clang++ \
    AR_x86_64_apple_darwin=x86_64-apple-darwin-ar \
    AR_aarch64_apple_darwin=aarch64-apple-darwin-ar \
    OSXCROSS_SDKROOT=/opt/osxcross/target/SDK/MacOSX${MACOS_SDK_VERSION}.sdk

WORKDIR /src

# Release-metadata ARGs/LABEL come last, deliberately: BUILDER_CREATED
# changes on every single release, and BuildKit's cache invalidates every
# layer *after* the one whose inputs changed — put this LABEL before the
# COPY/apt-get/rustup/symlink layers above (this is the slowest builder to
# rebuild, given the osxcross build in the first stage) and every release
# would fully redo all of them from scratch, even though nothing about the
# toolchain actually changed (verified directly against builders/linux-gnu,
# the same pattern this Dockerfile follows).
ARG BUILDER_VERSION=0.0.0
ARG BUILDER_REVISION=unknown
ARG BUILDER_CREATED=unknown

LABEL org.opencontainers.image.title="paws-builder-macos" \
      org.opencontainers.image.description="Rust cross-compilation builder for macOS targets (x86_64, aarch64) via osxcross" \
      org.opencontainers.image.source="https://github.com/mbround18/paws" \
      org.opencontainers.image.version="${BUILDER_VERSION}" \
      org.opencontainers.image.revision="${BUILDER_REVISION}" \
      org.opencontainers.image.created="${BUILDER_CREATED}" \
      io.paws.targets="x86_64-apple-darwin,aarch64-apple-darwin"
