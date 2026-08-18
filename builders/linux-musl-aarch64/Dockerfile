# syntax=docker/dockerfile:1.7
#
# Builder for aarch64-unknown-linux-musl. Based on messense/rust-musl-cross —
# see linux-musl-x86_64/Dockerfile's comment; this is the aarch64 counterpart
# (a separate base image, not an add-on package, since musl cross toolchains
# are built per-arch, not shared like glibc's cross-gcc packages are).

FROM messense/rust-musl-cross:aarch64-musl

WORKDIR /src

# Release-metadata ARGs/LABEL come last, deliberately — see
# builders/linux-gnu/Dockerfile's comment on why (BuildKit cache
# invalidation cascades downstream from whichever layer's inputs changed;
# BUILDER_CREATED changes every release). No expensive install layer exists
# in this Dockerfile today, but keeping the convention consistent means
# adding one later doesn't silently reintroduce the bug.
ARG BUILDER_VERSION=0.0.0
ARG BUILDER_REVISION=unknown
ARG BUILDER_CREATED=unknown

LABEL org.opencontainers.image.title="paws-builder-linux-musl-aarch64" \
      org.opencontainers.image.description="Rust cross-compilation builder for aarch64-unknown-linux-musl" \
      org.opencontainers.image.source="https://github.com/mbround18/paws" \
      org.opencontainers.image.version="${BUILDER_VERSION}" \
      org.opencontainers.image.revision="${BUILDER_REVISION}" \
      org.opencontainers.image.created="${BUILDER_CREATED}" \
      io.paws.targets="aarch64-unknown-linux-musl"
