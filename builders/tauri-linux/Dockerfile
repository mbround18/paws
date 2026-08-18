# syntax=docker/dockerfile:1.7
#
# Builder for Tauri apps on Linux (deb/rpm/AppImage bundling). Rust +
# Node (via NodeSource — the official Node image doesn't ship `cargo`, so
# it's simpler to add Node to a Rust base than the reverse) + the GTK/WebKit
# libraries the Tauri Linux backend links against, per
# https://tauri.app/start/prerequisites/#linux. Built through Dagger, not
# `docker build` directly — see builders/README.md.
#
# `xdg-utils` (for `xdg-open`) is required by Tauri's AppImage bundler even
# in a non-interactive build — a real bug this Dockerfile exists to have
# already hit and fixed once, not something every CI run should discover
# itself.

FROM rust:1-bookworm

ARG NODE_MAJOR=22

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update -qq && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        curl ca-certificates && \
    curl -fsSL "https://deb.nodesource.com/setup_${NODE_MAJOR}.x" | bash - && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        nodejs \
        libwebkit2gtk-4.1-dev \
        libappindicator3-dev \
        librsvg2-dev \
        patchelf \
        build-essential \
        file \
        xdg-utils

WORKDIR /src

# Release-metadata ARGs/LABEL come last, deliberately: BUILDER_CREATED
# changes on every single release, and BuildKit's cache invalidates every
# layer *after* the one whose inputs changed — put this LABEL before the
# apt-get layer above (by far the slowest step here) and every release
# would fully re-download and reinstall Node + the GTK/WebKit toolchain
# from scratch, even though nothing about it actually changed (verified
# directly against builders/linux-gnu, the same pattern this follows).
ARG BUILDER_VERSION=0.0.0
ARG BUILDER_REVISION=unknown
ARG BUILDER_CREATED=unknown

LABEL org.opencontainers.image.title="paws-builder-tauri-linux" \
      org.opencontainers.image.description="Rust + Node builder for Tauri apps on Linux (deb/rpm/AppImage bundling)" \
      org.opencontainers.image.source="https://github.com/mbround18/paws" \
      org.opencontainers.image.version="${BUILDER_VERSION}" \
      org.opencontainers.image.revision="${BUILDER_REVISION}" \
      org.opencontainers.image.created="${BUILDER_CREATED}" \
      io.paws.targets="tauri-linux"
