# Quickstart: Validating `paws ci --toolchain rust --coverage`

Prerequisites: `cargo build --workspace` succeeds; Docker daemon available (same requirement every
other `paws ci`/`paws docker` run already has).

## 1) Default (no `--coverage`) pipeline is unchanged

```bash
cargo test -p paws-rust
```

Covers the existing `pipeline_uses_the_rust_bookworm_image`/
`pipeline_runs_the_full_fmt_clippy_build_test_sequence_in_order` tests, now also asserting they
still pass unmodified with the new `coverage`/`builder_dir` parameters defaulted off (contracts
§1's byte-identical requirement).

## 2) `--coverage` on this repo itself (dogfooding, matching every other toolchain's own bar)

```bash
paws ci --toolchain rust --coverage
```

Expect: the existing `fmt`/`clippy`/`build`/`test` output, followed by a `cargo llvm-cov` summary
table (per-crate + `TOTAL` rows, Regions/Functions/Lines/Branches columns) — non-trivial
percentages, not 0% or absent.

## 3) `--coverage` rejected outside `--toolchain rust`

```bash
paws ci --toolchain node --coverage
```

Expect: a clear, non-zero-exit error before any pipeline runs (contracts §1) — mirrors `--targets`'s
existing out-of-`--toolchain go` rejection.

## 4) Incomplete-coverage fixture proves the tool is really measuring something

Automated: `examples/rust-coverage-fixture/` has a deliberately untested branch, and
`.github/workflows/ci.yaml`'s `ci-e2e` job runs `paws ci --toolchain rust --coverage` against it,
asserting the reported coverage is genuinely below 100% (and above 0%) — not just
running-and-reporting a fixed number. This lives in the CI workflow rather than a Rust-level
`#[test]`, because a test shelling out to `docker` directly (outside `paws-dagger`) would violate
`scripts/check-dagger-callsites.sh`'s SC-004/ADR-0001 lint, which only excepts
`crates/paws-docker/tests/`.

```bash
cargo build -p paws-cli
cd examples/rust-coverage-fixture && ../../target/debug/paws ci --toolchain rust --coverage
```

## 5) wasm project: `--coverage` is a no-op

```bash
paws ci --toolchain rust --coverage
```

against a fixture matching `is_wasm_project`'s detection (e.g. a `wasm-bindgen` dependency).
Expect: identical output to running without `--coverage` at all — no coverage step, no error
(research.md R5).

## Definition of done for this quickstart

- `cargo test --workspace` passes with zero failures.
- `docker buildx bake -f compose.yml rust` (or the equivalent local build) succeeds — proves
  `builders/rust/Dockerfile` itself builds cleanly, independent of a live `paws ci --coverage` run.
- Every scenario above is backed by an actual `#[test]` in `paws-rust`/`paws-cli-core` (tasks.md
  enumerates them 1:1) — this quickstart is a validation guide, not a substitute for the test
  suite.
