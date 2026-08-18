# paws

Run-anywhere CI/CD pipelines, backed by [Dagger](https://dagger.io), shipped as a single Rust binary.

Not tied to GitHub Actions: the same pipeline logic runs in CI or on your laptop.

## Origin

`paws` grew out of [gh-reusable](https://github.com/MBRound18/gh-reusable)'s reusable-workflow
library. That repo's `specs/002-reusable-rust-pipeline/` spec (copied here under `specs/`)
is the original inspiration for a first-class Rust pipeline contract — this project takes
that idea further: instead of adding Rust as one more supported language inside a
TypeScript-orchestrated system, the orchestrator itself is Rust.

## Layout

- `crates/paws-cli` — the `paws` binary. `clap`-based subcommands (`ci`, `docker`, `semver`,
  `audit`, `docs`, ...) are the narrative/user-facing layer.
- `crates/paws-core` — shared contract types (defaults, pipeline config shapes).
- `crates/paws-dagger` — wraps the `dagger` CLI. Deliberately **not** built on the
  `dagger-sdk` Rust crate yet — Dagger's own README marks that SDK experimental and
  "not for anything mission-critical." Pipeline logic goes through this crate so the
  day the SDK is trustworthy, only this crate needs to change.

## Principles

- **Reliability & testability first.** Every crate carries unit tests from day one;
  gh-reusable's existing actions are the reference implementations being ported over,
  not being reinvented.
- **Incremental SDK adoption.** New crates pilot the Rust `dagger-sdk` one at a time
  (starting with something low-stakes, e.g. semver) rather than betting the whole
  rewrite on an unstable dependency.
