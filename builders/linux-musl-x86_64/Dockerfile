# syntax=docker/dockerfile:1.7
#
# Builder for x86_64-unknown-linux-musl. Based on messense/rust-musl-cross —
# a maintained, known-good musl cross toolchain (the same family of images
# cross-rs/cross itself uses for musl targets) rather than hand-rolling a
# musl cross-gcc from scratch. Built through Dagger, not `docker build`
# directly — see builders/README.md.

FROM messense/rust-musl-cross:x86_64-musl

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

LABEL org.opencontainers.image.title="paws-builder-linux-musl-x86_64" \
      org.opencontainers.image.description="Rust cross-compilation builder for x86_64-unknown-linux-musl" \
      org.opencontainers.image.source="https://github.com/mbround18/paws" \
      org.opencontainers.image.version="${BUILDER_VERSION}" \
      org.opencontainers.image.revision="${BUILDER_REVISION}" \
      org.opencontainers.image.created="${BUILDER_CREATED}" \
      io.paws.targets="x86_64-unknown-linux-musl"
