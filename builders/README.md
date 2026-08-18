# Builders

Dedicated builder images, one directory per target family, each a plain `Dockerfile`. Most of
these are `paws release`'s own cross-compilation targets for building `paws` itself (see
`crates/paws-release/src/lib.rs`); `tauri-linux/`/`tauri-android/` are different — they're what
`paws ci --toolchain tauri`/`tauri-android` build *user* projects against (see
`crates/paws-tauri`), embedded into the `paws` binary rather than read from this directory at
runtime.

Every builder image is labeled with standard OCI annotations (`org.opencontainers.image.*`),
populated via `--build-args` at build time — see `../compose.yml`.

## Prebuilt images

`../compose.yml` builds all of these and tags them `ghcr.io/mbround18/paws-builders:<name>-
<version>` — a flat repo + tag, not one repo per builder, since Docker Hub doesn't support nested
repository paths the way GHCR does. `.github/workflows/release.yaml`'s `build-builders` job pushes
them to both GHCR and Docker Hub (same flat scheme on both) *before* the per-target build matrix
starts (`needs: [ci, build-builders]`). That matrix's `paws release` (via
`paws_dagger::remote_image_exists`/`prebuilt_image_candidate` in `crates/paws-release`) pulls the
matching tag rather than building any Dockerfile itself — deliberately pull-only, no local
`docker-build` fallback: rebuilding here would just re-pay for a build `build-builders` already
did once (the exact duplicated cost prebuilt images exist to remove) and would silently mask a
`build-builders` failure instead of surfacing it. If the tag isn't there, `paws release` fails
loudly rather than quietly falling back. `build-builders` also bootstraps the native `paws` binary
once (`cargo build --release -p paws-cli`) and uploads it as a build artifact, so the per-target
matrix downloads it instead of repeating that compile on every leg — a staged release, not N
independent ones.

This also means these Dockerfiles are no longer required to exist on disk wherever `paws release`
runs — the images are `paws`'s own, centrally published by `paws`'s own release pipeline, not
something every consuming repo needs to build itself.

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
- `tauri-android/` — JDK 17 + Android SDK (platform-tools, a platform, build-tools) + NDK + Rust's
  Android cross targets + Node, per https://tauri.app/start/prerequisites/#android. Used by `paws
  ci --toolchain tauri-android`; see `crates/paws-tauri`. There's no `tauri-ios/` and none is
  planned — iOS builds need real Xcode/`xcodebuild`, which Apple's license restricts to genuine
  macOS; no container image can provide that the way this one provides the Android SDK/NDK.
