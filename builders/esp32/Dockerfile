# syntax=docker/dockerfile:1.7
#
# Builder for `paws ci --toolchain esp32`: Rust + the ESP-IDF/`embuild` Rust
# toolchain (Xtensa-patched `rustc` + the `riscv32im*-esp-espidf` targets),
# installed via `espup` (the official ESP Rust toolchain installer — "shell
# to the real installer, don't reimplement it", same precedent
# `builders/tauri-android`'s Android SDK/NDK install and
# `crates/paws-provision`'s `install_rust`/`install_go` already follow) —
# not a from-scratch Xtensa LLVM/rustc build. Also installs `libclang`/
# `clang` (esp-idf-sys's `bindgen` step needs it — `LIBCLANG_PATH` is set
# here as a Dockerfile ENV, rather than the runtime auto-detection a local,
# non-containerized dev setup needs, since a container can just install the
# library at a known, fixed path), `python3`+pip (ESP-IDF's own
# `embuild`-driven CMake/component build shells out to Python), and
# `espflash` (`cargo install espflash` — used here for *packaging*
# artifacts, e.g. `espflash save-image`, not `espflash flash`, which needs a
# real USB-attached board no CI runner has).
#
# Joins the `tauri-android`/`flatpak`/`java` "needs multiple toolchains
# combined, no single public image provides it" category from
# docs/ROADMAP.md's builder-image policy — plain `rust:1-bookworm` has none
# of espup's Xtensa toolchain, libclang, or Python.

FROM rust:1-bookworm

ARG ESPUP_VERSION=0.15.1

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
        libclang-dev clang python3 python3-pip curl ca-certificates git

ENV LIBCLANG_PATH=/usr/lib/llvm-14/lib

RUN cargo install espup --version "${ESPUP_VERSION}" --locked && \
    espup install && \
    cargo install espflash --locked

WORKDIR /src

# Release-metadata ARGs/LABEL come last, deliberately — see
# builders/tauri-android/Dockerfile's header comment on why: BUILDER_CREATED
# changes on every release, and BuildKit invalidates every layer *after* the
# one whose inputs changed, so putting this before the espup/toolchain
# install layers above (by far the slowest step here, given the Xtensa
# toolchain + ESP-IDF download size) would re-download the whole thing on
# every single release even though nothing about it actually changed.
ARG BUILDER_VERSION=0.0.0
ARG BUILDER_REVISION=unknown
ARG BUILDER_CREATED=unknown

LABEL org.opencontainers.image.title="paws-builder-esp32" \
      org.opencontainers.image.description="Rust + espup ESP toolchain + libclang + espflash builder for paws ci --toolchain esp32" \
      org.opencontainers.image.source="https://github.com/mbround18/paws" \
      org.opencontainers.image.version="${BUILDER_VERSION}" \
      org.opencontainers.image.revision="${BUILDER_REVISION}" \
      org.opencontainers.image.created="${BUILDER_CREATED}" \
      io.paws.targets="esp32"
