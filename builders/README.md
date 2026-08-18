# Builders

Dedicated builder images `paws release` uses to cross-compile itself, one directory per target
family. Each is a plain `Dockerfile` built through Dagger (`dagger core host directory
--path=builders/<name> docker-build ...` — see `crates/paws-release/src/lib.rs`), not through
`docker build`/`cross` directly, so building `paws` never needs anything installed beyond the
`dagger` CLI itself (Dagger's own BuildKit-backed engine provides the layer caching, and its
`--platform` support provides the cross-arch execution needed to smoke-test the result).

Every builder image is labeled with standard OCI annotations (`org.opencontainers.image.*`),
populated via `--build-args` at build time from the release tag/commit being built — see
`paws_release::build_binary` in `crates/paws-release/src/lib.rs`.

- `linux-gnu/` — `x86_64-unknown-linux-gnu` (native) + `aarch64-unknown-linux-gnu` (via
  `gcc-aarch64-linux-gnu`).
- `linux-musl-x86_64/`, `linux-musl-aarch64/` — separate images (musl cross toolchains are
  per-arch, unlike glibc's shared cross-gcc packages), each based on
  [`messense/rust-musl-cross`](https://github.com/rust-cross/rust-musl-cross) (a maintained,
  known-good musl cross toolchain — not reimplemented from scratch).
- `windows-gnu/` — `x86_64-pc-windows-gnu` via `gcc-mingw-w64-x86-64`.
- `macos/` — `x86_64-apple-darwin` + `aarch64-apple-darwin` via
  [`osxcross`](https://github.com/tpoechtrager/osxcross). SDK fetched automatically from
  [`joseluisq/macosx-sdks`](https://github.com/joseluisq/macosx-sdks)' releases and verified
  against its published sha256sum — the same source `mbround18/setup-osxcross` and
  `joseluisq/docker-osxcross` use. Both targets build real, verified Mach-O binaries; neither is
  smoke-tested (no Mach-O execution environment available to `dagger`/Wine) — see
  `macos/README.md`.
