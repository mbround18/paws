# syntax=docker/dockerfile:1.7
#
# Builder for x86_64-pc-windows-gnu, via mingw-w64. Built through Dagger, not
# `docker build` directly — see builders/README.md.

FROM rust:1-bookworm

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
        gcc-mingw-w64-x86-64

RUN rustup target add x86_64-pc-windows-gnu

ENV CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc

WORKDIR /src

# Release-metadata ARGs/LABEL come last, deliberately: BUILDER_CREATED
# changes on every single release, and BuildKit's cache invalidates every
# layer *after* the one whose inputs changed — put this LABEL before the
# apt-get/rustup layers above and every release would fully re-download and
# reinstall the cross toolchain from scratch, even though nothing about it
# actually changed (verified directly against builders/linux-gnu, the same
# pattern this Dockerfile follows).
ARG BUILDER_VERSION=0.0.0
ARG BUILDER_REVISION=unknown
ARG BUILDER_CREATED=unknown

LABEL org.opencontainers.image.title="paws-builder-windows-gnu" \
      org.opencontainers.image.description="Rust cross-compilation builder for x86_64-pc-windows-gnu (mingw-w64)" \
      org.opencontainers.image.source="https://github.com/mbround18/paws" \
      org.opencontainers.image.version="${BUILDER_VERSION}" \
      org.opencontainers.image.revision="${BUILDER_REVISION}" \
      org.opencontainers.image.created="${BUILDER_CREATED}" \
      io.paws.targets="x86_64-pc-windows-gnu"
