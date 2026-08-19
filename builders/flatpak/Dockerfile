# syntax=docker/dockerfile:1.7
#
# Builder for Flatpak apps. Bakes flatpak + flatpak-builder + the Flathub
# freedesktop runtime/SDK/rust-stable extension in at a pinned version (the
# same convention builders/tauri-android uses for the Android SDK/NDK) —
# these are large (~2GB combined) and don't change per-build, so paying for
# them once per builder-image release, not once per `paws ci` run, is what
# actually makes this usable in CI.
#
# `flatpak-builder`'s sandboxed build step (a FUSE-backed rofiles overlay
# via bubblewrap) needs real elevated privileges — verified directly: with
# only `fuse3` installed and a bare `--device /dev/fuse`, the mount itself
# still fails ("Operation not permitted"); it needs Dagger's
# `--insecure-root-capabilities` on the `with-exec` that runs
# `flatpak-builder` (see `crates/paws-flatpak`), Dagger's equivalent of
# `docker run --privileged`. That's still routed entirely through Dagger
# (ADR-0001) — nothing here spawns `docker`/`cross` directly.
#
# `paws ci --toolchain flatpak` runs `flatpak-builder --build-only`, not a
# full bundle export — deliberately. A real, reproduced blocker: the
# metadata "finish" phase's own inner bubblewrap sandbox runs
# `appstream-compose`, a binary Debian bookworm doesn't ship anymore
# (superseded by `appstreamcli compose`) — and that inner sandbox doesn't
# see anything installed on the host PATH to work around it with a wrapper
# script either. `--build-only` stops right after compiling and installing
# into the Flatpak module tree, before that phase ever runs — which is the
# actual thing CI needs to verify (does the app build inside its own
# Flatpak sandbox), the same "build-verified, not a full package" scope
# `paws-release`'s macOS targets already use for a different reason.

FROM debian:bookworm-slim

ARG FLATPAK_RUNTIME_VERSION=25.08

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update -qq && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        flatpak flatpak-builder fuse3 ca-certificates git curl

RUN flatpak remote-add --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo && \
    flatpak install --system --noninteractive flathub \
        "org.freedesktop.Platform//${FLATPAK_RUNTIME_VERSION}" \
        "org.freedesktop.Sdk//${FLATPAK_RUNTIME_VERSION}" \
        "org.freedesktop.Sdk.Extension.rust-stable//${FLATPAK_RUNTIME_VERSION}"

WORKDIR /src

# Release-metadata ARGs/LABEL come last, deliberately — see
# builders/linux-gnu/Dockerfile's comment on why (BuildKit cache
# invalidation cascades downstream from whichever layer's inputs changed;
# BUILDER_CREATED changes every release, and the flatpak install above is
# by far the slowest step here).
ARG BUILDER_VERSION=0.0.0
ARG BUILDER_REVISION=unknown
ARG BUILDER_CREATED=unknown

LABEL org.opencontainers.image.title="paws-builder-flatpak" \
      org.opencontainers.image.description="flatpak + flatpak-builder + freedesktop runtime/SDK/rust-stable builder for Flatpak apps" \
      org.opencontainers.image.source="https://github.com/mbround18/paws" \
      org.opencontainers.image.version="${BUILDER_VERSION}" \
      org.opencontainers.image.revision="${BUILDER_REVISION}" \
      org.opencontainers.image.created="${BUILDER_CREATED}" \
      io.paws.targets="flatpak"
