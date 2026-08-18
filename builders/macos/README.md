# macOS builder

Cross-compiles `paws` for `x86_64-apple-darwin` and `aarch64-apple-darwin` using
[osxcross](https://github.com/tpoechtrager/osxcross). SDK acquisition and Cargo linker/rustflags
setup follow the same approach validated in
[mbround18/setup-osxcross](https://github.com/mbround18/setup-osxcross/blob/main/action.yml) (a
GitHub Action doing the equivalent setup): the macOS SDK is fetched from
[joseluisq/macosx-sdks](https://github.com/joseluisq/macosx-sdks)' GitHub releases — a
long-standing community mirror already relied on across the Rust cross-compilation ecosystem —
and verified against that release's published `sha256sum.txt` before osxcross is built against
it. No manual SDK download step needed.

## Building

```sh
dagger core host directory --path=builders/macos docker-build \
  --build-args=MACOS_SDK_VERSION=14.0 \
  ...
```

(`paws release --target x86_64-apple-darwin` wires this automatically — see
`crates/paws-release/src/lib.rs`'s `known_targets()`.)

`MACOS_SDK_VERSION` must be a tag at <https://github.com/joseluisq/macosx-sdks/releases>
(11.0+ for `aarch64-apple-darwin` support; defaults to `14.0`).

## Status

Cross-checked against two other real-world setups doing the same SDK fetch — 
[`mbround18/setup-osxcross`](https://github.com/mbround18/setup-osxcross/blob/main/action.yml) and
[`joseluisq/docker-osxcross`](https://github.com/joseluisq/docker-osxcross) (the base image behind
`joseluisq/rust-linux-darwin-builder`) — both pull from the same `joseluisq/macosx-sdks` releases.

Built and verified for real in this project's own development: `paws release --target
x86_64-apple-darwin` and `--target aarch64-apple-darwin` both produce genuine Mach-O 64-bit
executables (`file` confirms `Mach-O 64-bit x86_64/arm64 executable`), including `ring` (a
dependency with a C component, via `reqwest`'s `rustls-tls`) compiling and linking successfully —
that needed `CC_*`/`CXX_*`/`AR_*` env vars (not just `CARGO_TARGET_*_LINKER`) and an explicit
`OSXCROSS_SDKROOT` (osxcross's SDK autodetection parses its own wrapper binary's file name for a
darwin version suffix, which breaks once you symlink to a fixed unversioned name — direct
`OSXCROSS_SDKROOT` bypasses that) and `-fuse-ld=<osxcross's own ld>` (the wrapper doesn't select
its own bundled `ld` automatically here; without it, the system's plain GNU `ld` gets invoked, and
Mach-O-specific flags like `-dynamic` aren't ones it understands).

**Not verified**: actually *running* either binary. Dagger can't run a macOS container, and Wine
only emulates Windows PE, not Mach-O — no execution environment was available. `paws release`
reflects this honestly (`known_targets()`'s `smoke: None` for both darwin triples): it builds,
skips the smoke-test step with an explicit message, and packages the binary anyway. Also worth
noting: the link succeeds via `-undefined dynamic_lookup`, which permits unresolved symbols at
link time (resolved against the real macOS frameworks only at runtime) — a successful cross-link
is not a full guarantee of runtime correctness on real hardware the way it is for the natively-
verified Linux/Windows targets.
