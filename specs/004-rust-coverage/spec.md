# Feature Specification: `paws ci --toolchain rust --coverage`

## Summary

Add an opt-in `--coverage` flag to `paws ci` that, only when combined with `--toolchain rust`,
runs `cargo llvm-cov` inside the same Dagger pipeline `cargo test` already runs through, and
prints a coverage summary (region/function/line percentages, per the same table shape
`cargo llvm-cov --summary-only` already produces) to stdout at the end of the run. This is the
first pickup of the multi-language coverage-reporting goal tracked in `docs/ROADMAP.md`'s "Test
coverage reporting (`paws ci --coverage`)" section — deliberately scoped to Rust only; Node,
Python, Go, and Java/Kotlin each need their own native coverage tool wired the same way as
separate follow-up features.

## Clarifications

### Session 2026-08-23

- Q: Should `--coverage` replace the existing `cargo test --verbose` step, or run as an additional step alongside it? → A: Run as an additional step — `cargo test --verbose` stays exactly as it is today (unchanged pass/fail gate), and `cargo llvm-cov` runs separately afterward purely for the coverage report. Tests execute twice under `--coverage`; accepted as the cost of keeping the existing test step's behavior, exit code, and output completely untouched.
- Q: Should `cargo-llvm-cov`/`llvm-tools-preview` be installed at pipeline run time, or pre-baked into a new dedicated `builders/rust` image? → A: A new dedicated `builders/rust` image, pre-baked with `cargo-llvm-cov` and `llvm-tools-preview` on top of the same Rust toolchain base — the first exception to `docs/ROADMAP.md`'s "Rust doesn't need a builder image" policy, justified by per-run speed over the runtime-install alternative.

## Motivation and Problem Statement

`paws` has no coverage visibility today for any toolchain, including its own repo — confirmed
directly: no coverage tool is installed, configured, or referenced anywhere in this workspace
(`cargo-llvm-cov`/`cargo-tarpaulin` absent from every `Cargo.toml`, no coverage job in
`.github/workflows/ci.yaml`, no `Makefile` target). A maintainer (or `paws` itself, dogfooding)
currently has to install and run a coverage tool by hand, outside any `paws` pipeline, to answer
"how well-tested is this crate" — the same category of gap `paws ci`'s existing
build/lint/test steps already close for the underlying test run itself. This spec closes that gap
for Rust, the toolchain `paws` already dogfoods most heavily on its own codebase.

## Scope

### In scope

- A new `--coverage` boolean flag on `paws ci`, valid only in combination with
  `--toolchain rust` — mirrors the existing `--targets` flag's own gating precedent
  (`paws ci --toolchain go --targets ...`; `--targets` is already rejected outside
  `--toolchain go` with a clear error today).
- When set, `crates/paws-rust`'s Dagger pipeline runs `cargo llvm-cov --workspace --summary-only`
  (or the equivalent invocation resolved in plan.md) as an *additional* step after the existing
  `cargo test --verbose` step, which is left completely unchanged (see Clarifications, Session
  2026-08-23) — the coverage summary table is printed to stdout as part of the pipeline's normal
  output, on top of `cargo test`'s own existing output.
- A new `builders/rust` Dockerfile (see Clarifications, Session 2026-08-23) — the first Rust
  builder image `paws` ships — built on the same base `rust:1-bookworm` pipeline already uses,
  with `llvm-tools-preview` and `cargo-llvm-cov` pre-installed. `--coverage`'s pipeline uses this
  image instead of pulling `rust:1-bookworm` directly; the default (no `--coverage`) pipeline is
  unaffected and keeps pulling `rust:1-bookworm` exactly as it does today.
- Omitting `--coverage` (the default) MUST produce byte-identical pipeline behavior to today — no
  new step, no new image pull, no new output.

### Out of scope

- Node/Python/Go/Java/Kotlin coverage — tracked in `docs/ROADMAP.md`'s coverage table as
  follow-up features, each with its own native tool (`c8`/`istanbul`, `coverage.py`,
  `go test -cover`, JaCoCo).
- Uploading a coverage report anywhere external (Codecov, Coveralls, etc.) — this spec is
  local/CI-log visibility only.
- Failing the build (or gating merge) on a coverage threshold — `--coverage` only reports; it
  never changes the pipeline's pass/fail outcome by itself in this first cut.
- Exporting a machine-readable report file (lcov, Cobertura XML, `.profraw`/`.profdata`) out of
  the Dagger container — this spec is a printed summary only; a follow-up can add
  `--coverage-report <path>`-style export once there's a real consumer for the file (e.g. an
  upload step).
- Any change to `cargo test`'s own existing behavior, flags, or output when `--coverage` is not
  set.

## Affected Contracts

- **`paws ci` CLI contract**: a new flag (`--coverage`) added to the existing `CiArgs`/subcommand,
  gated to `--toolchain rust` only. Additive, non-breaking — matches the constitution's
  Development Workflow guidance on pre-1.0 backward compatibility (same additive-flag shape as
  this repo's `003-release-parity-docker` feature's `--tag-rollup`/`--tag-*` flags).
- **`paws-rust` pipeline-args contract**: `crates/paws-rust`'s Dagger pipeline-argument builder
  (whatever function currently assembles the `fmt`/`clippy`/`build`/`test` chain) gains a new
  opt-in branch for the coverage step. Existing callers passing no coverage option see unchanged
  output.
- **Downstream consumer behavior**: no default change for any existing `paws ci --toolchain rust`
  caller. A caller that adds `--coverage` gets an additional pipeline step and additional stdout
  output; nothing else in the pipeline changes.
- **No `gh-reusable` contract to stay in parity with** — `gh-reusable`'s own `rustBuildAndTest`
  Dagger function (the one `paws-rust` already fully ported, per `docs/ROADMAP.md`) has no
  coverage step at all; this is new, `paws`-native capability, not a port.

## Runtime and Defaults Impact

- No new `paws-core::PipelineDefaults` fields — this is a pure CLI-flag-gated pipeline branch, no
  shared runtime default involved (matches how `--targets`/`--with-latest`-style flags needed
  none either).
- Container implications: a new `builders/rust` image (Clarifications, Session 2026-08-23) —
  `rust:1-bookworm`'s base plus `rustup component add llvm-tools-preview` and `cargo install
  cargo-llvm-cov`, baked in at image-build time rather than per-pipeline-run. This is the first
  exception to `docs/ROADMAP.md`'s "Rust doesn't need a builder image" note (`paws-rust` has
  pulled `rust:1-bookworm` directly, no dedicated Dockerfile, until now) — plan.md resolves the
  image's build/publish/versioning mechanics (matching the existing `builders/*` precedent:
  `docs/ROADMAP.md`'s "How a new stack gets added" step 3 / `builders/README.md`).
- No change to the default (no-`--coverage`) pipeline's runtime footprint at all — it keeps
  pulling `rust:1-bookworm` directly, exactly as today.

## Security and Permissions Impact

- No new secrets, tokens, or permissions — `cargo llvm-cov` runs entirely inside the same
  already-authenticated (or unauthenticated, for a plain build/test run) Dagger container
  `cargo test` already runs in. No registry push, no external upload, no new attack surface.
- No scanner/policy behavior change — `paws audit`'s scanner suite is unrelated to this flag.

## Validation Plan

- `paws ci --toolchain rust --coverage`, run for real (dogfooding `paws` on its own repo, matching
  every other toolchain's own "verified for real, end to end" bar in `docs/ROADMAP.md`), produces
  a coverage summary in stdout with a non-trivial (not 0%, not silently absent) percentage for at
  least one crate.
- A fixture with a deliberately incomplete test suite (e.g. a function with an untested branch)
  shows a coverage percentage measurably below 100% — proves the tool is actually measuring
  something, not just running and reporting a fixed/fake number.
- With `--coverage` set, a failing test still fails the pipeline via the existing `cargo test
  --verbose` step (unchanged, per Clarifications) — before `cargo llvm-cov` even runs, since
  `cargo test`'s own step keeps its existing fail-fast position in the pipeline chain. The
  coverage step is not a new place pipeline failure can originate from in a way `cargo test`
  didn't already cover.
- `paws ci --toolchain rust` (no `--coverage`) produces byte-identical pipeline args/output to
  today — a regression test comparable to `003-release-parity-docker`'s SC-001 fixed-snapshot
  approach for `paws-docker`.
- `--coverage` combined with any `--toolchain` other than `rust` fails with a clear, actionable
  error (mirroring `--targets`'s existing out-of-`--toolchain go` rejection message shape).
- `cargo test --workspace` continues to pass with zero failures workspace-wide (Constitution
  Principle V).

## Rollout and Rollback

- Ships as a pure opt-in flag — no coordinated migration, no default-behavior change, zero impact
  on any existing `paws ci --toolchain rust` caller that doesn't pass `--coverage`.
- If a regression is found, a consumer stops passing `--coverage` — no `paws` version rollback
  required.
- Node/Python/Go/Java/Kotlin coverage support (tracked in `docs/ROADMAP.md`) can each land as
  independent follow-up features once this one's CLI-contract shape (flag name, gating pattern,
  output format) is proven out — this spec deliberately doesn't pre-commit those follow-ups to
  any particular contract shape beyond "look like this one."
