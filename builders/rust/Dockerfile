# syntax=docker/dockerfile:1.7
#
# Builder for `paws ci --toolchain rust --coverage`: the same `rust:1-bookworm` base every plain
# `paws ci --toolchain rust` run already pulls directly, plus `cargo-llvm-cov` and the
# `llvm-tools-preview` rustup component it needs, pre-baked so a `--coverage` run doesn't pay a
# ~30s fresh-install cost on every invocation (see specs/004-rust-coverage/spec.md's
# Clarifications). Built through Dagger, not `docker build` directly -- see builders/README.md.
#
# Only used when `--coverage` is set -- the default (no-flag) `paws ci --toolchain rust` pipeline
# keeps pulling `rust:1-bookworm` directly, unaffected by this image's existence
# (crates/paws-rust's `dagger_pipeline_args` only swaps to this image's `docker-build` chain when
# `coverage` is true; see specs/004-rust-coverage/research.md R2).

FROM rust:1-bookworm

RUN rustup component add llvm-tools-preview \
    && cargo install cargo-llvm-cov --locked

ARG BUILDER_VERSION=0.0.0
ARG BUILDER_REVISION=unknown
ARG BUILDER_CREATED=unknown

LABEL org.opencontainers.image.title="paws-builder-rust" \
      org.opencontainers.image.description="rust:1-bookworm + cargo-llvm-cov for paws ci --toolchain rust --coverage" \
      org.opencontainers.image.source="https://github.com/mbround18/paws" \
      org.opencontainers.image.version="${BUILDER_VERSION}" \
      org.opencontainers.image.revision="${BUILDER_REVISION}" \
      org.opencontainers.image.created="${BUILDER_CREATED}" \
      io.paws.targets="rust"
