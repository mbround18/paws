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
GTK/WebKit). Each matrix leg runs `docker buildx bake -f compose.yml --push <builder>` — `buildx
bake` directly, not `docker compose build --push`. Two real, reproduced bugs led here:

1. **Both registries under `image:` + `build.tags` split, not both under `build.tags`.** Looked
   right at first (`docker compose build --push` pushed *a* manifest), but `docker buildx bake
   --print` against the generated bake definition showed Compose's compose→bake translation
   silently drops the top-level `image:` field whenever `build.tags` is also set — only the Docker
   Hub ref (`build.tags`) ever actually got pushed; GHCR (`image:`) only ever received the
   registry *cache*, never the image. Fixed by putting both refs under `build.tags` and dropping
   `image:` entirely — verified directly: the bug reproduced with the split, and a single push
   landed both refs once both were under `build.tags`.
2. **`docker buildx bake` directly, not `docker compose build --push`.** On the runner's Docker
   Compose version (v2.38.2) specifically, `docker compose build --push` silently loaded the built
   image into the local Docker daemon instead of pushing it at all — no error, exit 0, only the
   registry cache landed, not the image. Didn't reproduce locally against the same `compose.yml`.
   Calling `buildx bake` directly (still reading this same `compose.yml` as its bake definition)
   reliably took the correct push path instead.

`build-builders` also re-verifies both pushes actually landed (`docker buildx imagetools
inspect`) as a belt-and-suspenders check, given how quietly both bugs above failed — catching a
repeat loudly instead of it surfacing later as a confusing "image not found" in `paws release`.

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
- `flatpak/` — `flatpak` + `flatpak-builder` + the Flathub `org.freedesktop.Platform`/`Sdk`/
  `Sdk.Extension.rust-stable` runtime baked in at a pinned version (~2GB combined; the same
  reason `tauri-android` bakes its SDK/NDK in rather than installing at pipeline-run time). Used
  by `paws ci --toolchain flatpak`; see `crates/paws-flatpak`. Two real constraints shaped this:
  `flatpak-builder`'s sandboxed build needs `--insecure-root-capabilities` on the `with-exec`
  (verified: `fuse3` + a bare `--device /dev/fuse` aren't enough on their own — the FUSE mount
  itself fails with "Operation not permitted" without it — still routed entirely through Dagger,
  ADR-0001), and `paws ci` only runs `flatpak-builder --build-only`, not a full bundle export —
  the metadata "finish" phase's own *inner* sandbox needs `appstream-compose`, a binary Debian
  bookworm no longer ships, and that inner sandbox can't be handed a host-side workaround either.
  Verified for real, end to end, against a genuine app (`mbround18/oled-wallpaper`'s actual
  Flatpak manifest, a heavy wgpu/winit GUI app), not a synthetic fixture.
