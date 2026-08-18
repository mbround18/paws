# syntax=docker/dockerfile:1.7
#
# Builder for x86_64-unknown-linux-gnu (native) and aarch64-unknown-linux-gnu
# (cross, via a Debian cross-gcc). Built through Dagger, not `docker build`
# directly — see builders/README.md.

FROM rust:1-bookworm

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

RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update -qq && \
    DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        gcc-aarch64-linux-gnu \
        libc6-dev-arm64-cross

RUN rustup target add x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu

ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc

WORKDIR /src
