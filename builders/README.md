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
starts (`needs: [ci, bootstrap, build-builders]`). That matrix's `paws release` (via
`paws_dagger::remote_image_exists`/`prebuilt_image_candidate` in `crates/paws-release`) pulls the
matching tag rather than building any Dockerfile itself — deliberately pull-only, no local
`docker-build` fallback: rebuilding here would just re-pay for a build `build-builders` already
did once (the exact duplicated cost prebuilt images exist to remove) and would silently mask a
`build-builders` failure instead of surfacing it. If the tag isn't there, `paws release` fails
loudly rather than quietly falling back. A separate `bootstrap` job builds the native `paws`
binary once (`cargo build --release -p paws-cli`) and uploads it as a build artifact, so the
per-target matrix downloads it instead of repeating that compile on every leg — a staged release,
not N independent ones.

`build-builders` itself is a matrix — one runner per builder, not all 7 on a single runner.
Building them concurrently on one machine exhausted a real GitHub-hosted runner's disk ("no space
left on device": every builder here is a full toolchain image — JDK+Android SDK/NDK, osxcross,
GTK/WebKit — and `docker compose build`'s default behavior duplicates each one's storage once for
BuildKit's own cache and again importing it into the local Docker daemon). Each matrix leg runs
`docker compose build --push <builder>`, which pushes to both registries in the same build —
`image:` is the GHCR ref, `compose.yml`'s `build.tags` adds the Docker Hub one — rather than a
separate copy step, verified directly against a pair of local test registries.

`build-builders` also re-verifies both pushes actually landed (`docker buildx imagetools
inspect`), rather than trusting the build step's exit code alone: a real, reproduced bug had
`docker compose build --push` (with only `image:` set, no `build.tags`) silently load the built
image into the runner's local Docker daemon instead of pushing it — no error, exit 0, only the
BuildKit registry *cache* actually landed in the registry. That exact failure didn't reproduce
locally against the same `compose.yml` pattern (a Docker Compose version difference is the leading
suspect — the runner ran v2.38.2), so this doesn't assume `build.tags` fully closes it; the
verification step is what actually catches a repeat, loudly, instead of it surfacing later as a
confusing "image not found" in `paws release`.

This also means these Dockerfiles are no longer required to exist on disk wherever `paws release`
runs — the images are `paws`'s own, centrally published by `paws`'s own release pipeline, not
something every consuming repo needs to build itself.

Each service in `compose.yml` also sets `cache_from`/`cache_to` (BuildKit's registry cache
exporter, `type=registry,mode=max`, one `cache-<name>` tag per builder on GHCR) — this is what
actually lets build cache survive *between* separate `release.yaml` runs on GitHub's ephemeral
runners, not just within one. The `builders/*/Dockerfile` ARG/LABEL reorder above only protects
Docker's local layer cache, which never persists across runners anyway; the registry cache is what
makes that matter. Verified for real against a local test registry: a completely fresh `buildx`
builder with no local cache at all still hit `CACHED` on every layer by importing it, even with
`BUILDER_CREATED` changed between builds. Requires the `docker-container` buildx driver — the
default `docker` driver doesn't support the registry cache exporter (confirmed directly: a build
against it silently exports nothing) — which is why `release.yaml`'s `build-builders` job runs
`docker/setup-buildx-action` before `docker compose build`.

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
