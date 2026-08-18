---
status: "accepted"
date: 2026-08-18
decision-makers: "mbround18, Claude (pairing)"
---

# Route all container execution through Dagger, not direct `docker`/`cross` spawns

## Context and Problem Statement

`paws` exists to be "run-anywhere CI/CD pipelines, backed by Dagger" — a single Rust binary that
replaces `gh-reusable`'s TypeScript/GitHub-Actions-only pipeline surface. The project's own
constitution (Principle II, "Subprocess-First Dagger Access, Single Call Site") already commits
to this: Dagger is invoked exclusively as a subprocess through `paws-dagger::call`, enforced by a
grep-based CI lint (SC-004) that fails the build if `Command::new("dagger")` appears anywhere
outside `crates/paws-dagger`.

When `crates/paws-release` was first built (`paws release`: cross-compile `paws` for multiple
targets, smoke-test the result, package it, and publish it to a GitHub Release — the mechanism
`paws` uses to build and release itself), it worked and was verified end-to-end, but it shelled
directly to `cargo`/[`cross`](https://github.com/cross-rs/cross) (itself Docker-based) for
cross-target builds, and to raw `docker run`, `docker/setup-qemu-action`, and a Wine container for
smoke-testing the results. SC-004's lint didn't catch this — it only checks for `dagger` call
sites, not `docker`/`cross` ones — but it's the same problem in spirit: a second, uncontrolled
container-execution path alongside the one Dagger is supposed to own, and it meant `paws release`
needed Docker, `cross`, a QEMU setup action, and Wine independently installed, on top of `dagger`
itself.

How should `paws` build and verify cross-target release binaries (Linux gnu/musl × x86_64/
aarch64, Windows via mingw, macOS via osxcross) without reintroducing that second execution path?

## Decision Drivers

* Constitution Principle II: Dagger is meant to be the single container-execution seam; a crate
  quietly bypassing it defeats the point even if no automated check catches it.
* A user running `paws release` shouldn't need Docker/`cross`/Wine/a QEMU setup step installed
  independently of `dagger` — that's exactly the fragmentation `paws` exists to eliminate.
* Need reliable multi-arch (aarch64) and cross-OS (Windows PE via Wine) execution to actually
  *smoke-test* a cross-compiled binary before publishing it, not just trust that it compiled.
* Build caching should come for free, not require maintaining a second cache mechanism
  (`actions/cache`, a registry mirror, ...) alongside whatever Dagger already does.
* `dagger-sdk` (the Rust SDK) remains explicitly out of scope per the constitution's Principle
  III and the project's `Assumptions` — it's marked experimental upstream, and adopting it is
  deferred until the `paws-semver` pilot has run in production for a trial period.

## Considered Options

* Keep `cargo`/`cross` for builds and `docker run`/QEMU-action/Wine for smoke tests (status quo)
* Route builds and smoke tests through `dagger core` — moduleless `dagger core <chain>` pipelines
  against dedicated `./builders/*` Dockerfiles, with no custom Dagger module required
* Author a custom Dagger module (TypeScript, alongside `gh-reusable`'s existing one) exposing a
  `paws`-specific build/release function

## Decision Outcome

Chosen option: "Route builds and smoke tests through `dagger core`", because it's the only option
that satisfies the single-call-site principle without adopting `dagger-sdk` (ruled out by Decision
Driver 5) or standing up and maintaining a second TypeScript module (ruled out below). Confirmed
by direct experimentation that `dagger core <chain>` (`host directory ... docker-build`,
`with-mounted-directory`, `with-exec`, `file`, `export`, `container --platform=...`) works without
a custom module — `paws-dagger` gained one new function, `core()`, alongside the existing
module-based `call()`, and both remain the only two places that spawn the `dagger` process.

### Consequences

* Good, because `paws-dagger` stays the single seam that spawns a container-runtime process —
  SC-004's lint claim ("no direct container-engine spawns outside this crate") is actually true
  again, not true-in-letter-only.
* Good, because a user only needs the `dagger` CLI installed to run `paws release` — no `cross`,
  no `docker/setup-qemu-action`, no separate Wine setup. `.github/workflows/release.yaml` shrank
  accordingly (removed the `cross` install step, the QEMU-action step, and the Wine step).
* Good, because Dagger's own BuildKit-backed engine provides layer caching for free — confirmed
  directly: an unchanged `./builders/*` Dockerfile and unchanged mounted source are reported
  `CACHED` on a second `dagger core` invocation, with no cache configuration added on `paws`'s
  side.
* Good, because Dagger's `--platform` support (backed by the host's QEMU `binfmt_misc`
  registration, the same mechanism `docker run --platform` and `docker/setup-qemu-action` use
  under the hood) covers aarch64 execution, and a Wine-enabled base image run *as* a Dagger
  container covers the Windows target — so "smoke-test a binary for a different CPU/OS than the
  runner" stays inside the same one mechanism instead of needing a different tool per target.
* Neutral, because `dagger core` is documented upstream as "currently under development and may
  change in the future" — this is a real dependency on a less-stable corner of the Dagger CLI
  than the module-based `dagger call` path `paws`'s other subcommands use.
* Bad, because it added real debugging surface: getting `./builders/macos` working through this
  path required tracking down osxcross-specific issues (missing `CC_*`/`CXX_*`/`AR_*` env vars
  for C-component dependencies like `ring`; `OSXCROSS_SDKROOT` needing to be set explicitly once
  the compiler wrapper's SDK-autodetection-by-filename broke from symlinking it to a fixed name;
  the wrong linker being invoked without an explicit `-fuse-ld=`) that a `docker build`/`cross`
  path would have hit too, but that were harder to iterate on through `dagger core`'s more
  verbose CLI invocations than a plain `docker build`/`docker run` loop would have been.

### Confirmation

Enforced by `scripts/check-dagger-callsites.sh`, extended alongside this ADR to also fail on
`Command::new("docker")`/`Command::new("cross")` outside `crates/paws-dagger` (the same script
SC-004 already used for `dagger` call sites), with one deliberate, narrowly-scoped exception:
`crates/paws-docker/tests/`'s e2e suite shells to `docker` on purpose, to validate `paws-docker`'s
own facts-resolution logic against a real Docker daemon — that's testing `paws`, not `paws`
executing a pipeline, so it's excluded rather than routed through `paws-dagger`. Also manually
confirmed working end-to-end for all 7 targets in
`paws_release::known_targets()`: `paws release --target <triple> --tag v0.0.1-prerelease.1
--no-upload` run through the real `paws` binary for each of `x86_64-unknown-linux-gnu`,
`aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`,
`x86_64-pc-windows-gnu` (build + smoke test + package all succeeded, zero direct `docker`/`cross`/
`wine` process spawns from `paws` itself), and `x86_64-apple-darwin`/`aarch64-apple-darwin`
(build + package succeeded; smoke-testing these two isn't possible in any environment available
here, so `paws release` reports that honestly rather than skipping silently or lying about it —
see `builders/macos/README.md`).

## Pros and Cons of the Options

### Keep `cargo`/`cross` + `docker run`/QEMU-action/Wine (status quo)

The original, already-shipped-once approach: `cross build --target ...` for cross-compilation,
`docker run --platform=... <image> <binary> --version` for aarch64 smoke tests (after registering
QEMU emulation via `docker/setup-qemu-action` in CI), and a Wine-enabled Docker image run directly
for the Windows target.

* Good, because it's the most conventional, best-documented path in the wider Rust
  cross-compilation ecosystem — most examples and prior art (including the projects reviewed
  while building `builders/macos/`) use exactly this.
* Good, because debugging is more direct: `docker build`/`docker run` errors are usually
  easier to read than the equivalent `dagger core` chain's output.
* Bad, because it bypasses `paws-dagger` — the exact problem this ADR exists to resolve.
* Bad, because it requires `cross`, `docker/setup-qemu-action`, and Wine installed/configured
  independently of `dagger`, fragmenting `paws`'s own toolchain requirements.
* Bad, because it needs its own caching strategy (`actions/cache` keyed on target, `Swatinem/
  rust-cache`, ...) rather than getting it from the same engine that's already caching `paws`'s
  other Dagger-routed builds.

### `dagger core` moduleless pipelines against `./builders/*` (chosen)

See "Decision Outcome" above.

* Good, because it's fully consistent with the constitution's single-call-site principle.
* Good, because no custom Dagger module needs authoring or maintaining — `./builders/*` are
  plain Dockerfiles, the lowest-ceremony way to define a build environment, and `dagger core`
  chains them directly from the CLI.
* Good, because it needs nothing beyond the `dagger` CLI on the caller's machine.
* Neutral, because `dagger core` is an explicitly experimental corner of the Dagger CLI
  ("currently under development and may change in the future," per its own `--help` output).
* Bad, because the CLI invocations are long, positional-argument-chain pipelines
  (`dagger core host directory --path=... docker-build --build-args=... with-mounted-directory
  --path=/src --source=... with-exec --args=... file --path=... export --path=...`) that are
  less immediately readable than an equivalent `docker build && docker run` pair, which cost real
  debugging time while getting `builders/macos/` working.

### Custom Dagger module (TypeScript)

Author a new `@func()` (or extend `gh-reusable`'s existing `packages/dagger-module`) exposing a
`paws`-specific `buildRelease(target, ...)` function, called the same way `paws ci`/`paws docker`/
`paws audit` already call into `gh-reusable`'s module via `paws-dagger::call`.

* Good, because it would reuse the exact same interim-wiring pattern already established for
  `ci`/`docker`/`audit`, rather than introducing a second Dagger-invocation shape (`core()`
  alongside `call()`).
* Good, because TypeScript pipeline definitions are more conventional Dagger usage than raw CLI
  chains, and easier to read/maintain than a long `dagger core` invocation.
* Bad, because it reintroduces exactly the TypeScript-orchestration dependency `paws` exists to
  get away from (see `README.md`'s "Origin" section) — `paws`'s own release mechanism would
  depend on `gh-reusable` (or a new TS module) staying in sync and reachable, the same risk
  already flagged and mitigated elsewhere in this project by pinning `GH_REUSABLE_DAGGER_MODULE`
  to a known-good commit after its floating `main` branch was found broken.
* Bad, because it's meaningfully more work to stand up (a new module, its own `dagger.json`, a
  place to host/version it) than writing a Dockerfile, for a capability (`docker-build` +
  `with-exec` + `export`) Dagger's own core types already provide directly.

## More Information

Implementation: `crates/paws-dagger/src/lib.rs`'s `core()` function; `crates/paws-release/src/
lib.rs`'s `build_binary`/`smoke_test`; `./builders/*` Dockerfiles;
`.github/workflows/release.yaml`. Tracked in `specs/001-paws-core-cli/tasks.md` under
task group 10 (`paws release` — cross-target build + GitHub Release publish), including the
"Revised after review" section documenting the same before/after this ADR summarizes.
