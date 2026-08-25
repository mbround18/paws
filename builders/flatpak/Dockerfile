# syntax=docker/dockerfile:1.7
#
# Builder for Flatpak apps. Bakes flatpak + flatpak-builder + the Flathub
# freedesktop runtime/SDK/rust-stable extension in at a pinned version (the
# same convention builders/tauri-android uses for the Android SDK/NDK) —
# these are large (~2GB combined) and don't change per-build, so paying for
# them once per builder-image release, not once per `paws ci` run, is what
# actually makes this usable in CI.
#
# Base is `ubuntu:26.04`, not `debian:bookworm-slim` — deliberately, and not
# arbitrarily. Real root cause, found by comparing against a genuine
# passing CI run (mbround18/oled-wallpaper's release.yml, which does a full
# `flatpak-builder` + `flatpak build-bundle` on `ubuntu-latest`): Debian
# bookworm ships `flatpak-builder 1.2.3`, which still shells out to a
# standalone `appstream-compose` binary during the metadata "finish" phase
# — a binary that no longer exists in modern `appstream` packaging
# (superseded by `appstreamcli compose`), so that phase fails with "No such
# file or directory" inside its own inner bubblewrap sandbox (which doesn't
# see anything installed on the outer container's PATH either, so a
# host-side wrapper script doesn't help). Every current Ubuntu release ships
# `flatpak-builder` >= 1.4, new enough to call `appstreamcli compose`
# directly (`appstreamcli` itself is already pulled in as a `flatpak-builder`
# dependency — no separate `appstream-compose` package needed at all, that
# was a red herring: it doesn't provide the standalone binary either, on any
# distro checked) — confirmed directly, not guessed, by installing multiple
# flatpak-builder versions side by side and diffing their actual behavior.
# It was never about the distro, only the `flatpak-builder` version each one
# happens to package. This closes the *missing-binary* failure specifically;
# a full bundle export (not just `--build-only`) still hits a separate,
# unresolved `appstreamcli compose` runtime difference under this pipeline's
# root context — see `crates/paws-flatpak`'s doc comment. `--build-only`
# remains the supported scope.
#
# `xvfb` is included for GUI runtime testing (launching a built Flatpak app
# headlessly to verify it actually starts, not just that it compiles) —
# also the shared building block for the same headless-display need
# Playwright/browser e2e testing has, independent of Flatpak entirely.
#
# `flatpak-builder`'s sandboxed build step (a FUSE-backed rofiles overlay
# via bubblewrap) needs real elevated privileges — verified directly: with
# only `fuse3` installed and a bare `--device /dev/fuse`, the mount itself
# still fails ("Operation not permitted"); it needs Dagger's
# `--insecure-root-capabilities` on the `with-exec` that runs
# `flatpak-builder` (see `crates/paws-flatpak`), Dagger's equivalent of
# `docker run --privileged`. That's still routed entirely through Dagger
# (ADR-0001) — nothing here spawns `docker`/`cross` directly.

FROM ubuntu:26.04

ARG FLATPAK_RUNTIME_VERSION=25.08

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
        flatpak flatpak-builder fuse3 xvfb ca-certificates git curl

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
      org.opencontainers.image.description="flatpak + flatpak-builder + xvfb + freedesktop runtime/SDK/rust-stable builder for Flatpak apps" \
      org.opencontainers.image.source="https://github.com/mbround18/paws" \
      org.opencontainers.image.version="${BUILDER_VERSION}" \
      org.opencontainers.image.revision="${BUILDER_REVISION}" \
      org.opencontainers.image.created="${BUILDER_CREATED}" \
      io.paws.targets="flatpak"
