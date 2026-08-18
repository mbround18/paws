# Builders

Dedicated builder images, one directory per target family, each a plain `Dockerfile` built
through Dagger (`dagger core host directory --path=builders/<name> docker-build ...`), not through
`docker build`/`cross` directly — so nothing beyond the `dagger` CLI itself is ever required
(Dagger's own BuildKit-backed engine provides the layer caching, and its `--platform` support
provides the cross-arch execution `paws release`'s smoke tests need). Most of these are
`paws release`'s own cross-compilation targets for building `paws` itself (see
`crates/paws-release/src/lib.rs`); `tauri-linux/` is different — it's what `paws ci --toolchain
tauri` builds *user* Tauri projects against (see `crates/paws-tauri`), embedded into the `paws`
binary rather than read from this directory at runtime.

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
- `tauri-linux/` — Rust + Node (via NodeSource) + the GTK/WebKit libraries Tauri's Linux backend
  links against, per https://tauri.app/start/prerequisites/#linux. Used by `paws ci --toolchain
  tauri`, not `paws release`; see `crates/paws-tauri`.
