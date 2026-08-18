# syntax=docker/dockerfile:1.7
#
# Builder for x86_64-unknown-linux-gnu (native) and aarch64-unknown-linux-gnu
# (cross, via a Debian cross-gcc). Built through Dagger, not `docker build`
# directly — see builders/README.md.

FROM rust:1-bookworm

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update -qq && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        gcc-aarch64-linux-gnu \
        libc6-dev-arm64-cross

RUN rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu

ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc

WORKDIR /src

# Release-metadata ARGs/LABEL come last, deliberately: BUILDER_CREATED
# changes on every single release, and BuildKit's cache invalidates every
# layer *after* the one whose inputs changed — put this LABEL before the
# apt-get/rustup layers above and every release would fully re-download and
# reinstall the cross toolchain from scratch, even though nothing about it
# actually changed. Verified directly: with the metadata block up front,
# rebuilding with only BUILDER_CREATED changed re-ran the apt-get layer in
# full instead of hitting cache.
ARG BUILDER_VERSION=0.0.0
ARG BUILDER_REVISION=unknown
ARG BUILDER_CREATED=unknown

LABEL org.opencontainers.image.title="paws-builder-linux-gnu" \
      org.opencontainers.image.description="Rust cross-compilation builder for glibc Linux targets (x86_64, aarch64)" \
      org.opencontainers.image.source="https://github.com/mbround18/paws" \
      org.opencontainers.image.version="${BUILDER_VERSION}" \
      org.opencontainers.image.revision="${BUILDER_REVISION}" \
      org.opencontainers.image.created="${BUILDER_CREATED}" \
      io.paws.targets="x86_64-unknown-linux-gnu,aarch64-unknown-linux-gnu"
