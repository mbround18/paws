# Contributing

## Before opening a PR

`.github/workflows/ci.yaml` is the actual gate — run the same checks locally before pushing:

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
./scripts/check-dagger-callsites.sh
```

The last one enforces [ADR 0001](docs/adr/0001-route-container-execution-through-dagger.md): no
`Command::new("dagger"/"docker"/"cross")` outside `crates/paws-dagger` (`paws-docker`'s e2e tests
excepted). See [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) for crate layout and architecture, and
[`docs/ROADMAP.md`](docs/ROADMAP.md) for what's planned versus already wired.

## Rust CI acceleration and cache strategy

For Rust-heavy builds, the repo already uses `Swatinem/rust-cache` in CI and keeps the cache keyed
by the lockfile so dependency layers are reused across runs. That is the fastest low-risk win for
workspace builds.

For Dockerized Rust builds, prefer a `cargo-chef` pattern when you want to split dependency planning
from the final compile step. The repo keeps a ready-made helper in the root `Makefile` for that
workflow:

```sh
make cargo-chef
make cargo-chef-plan
make cargo-chef-cook
```

This is especially useful when building release images or containerized toolchains, because it keeps
`Cargo.lock`/dependency resolution stable while letting the Docker layer cache separate the
"dependency recipe" phase from the final compile. For this repo, the Dagger-first execution model
remains the primary path, but the `cargo-chef` plan is useful for any Docker image that builds Rust
code directly.

## Delegating implementation work to an AI agent/subagent

This repo is developed heavily with Claude Code, including via forked subagents that implement
whole features. Keep their token cost proportional to the work:

1. **One deliverable per fork, not a bundle.** A single fork call that combines multiple
   feature-sized chunks (e.g. "add X, add Y, and close these two test gaps") pays for all of it
   before any of it can be reviewed or corrected. Split into separate fork calls per deliverable
   even if it means slightly more prompt-writing overhead — it lets a cheap piece finish and get
   checked before paying for an expensive one.
2. **Point at exact files/functions/line numbers already verified, never "explore and figure it
   out."** A fork with a vague prompt re-explores the codebase from scratch inside the fork
   (re-reading `main.rs`, re-deriving API shapes already looked up). A prompt with concrete
   paths/line numbers/API calls already confirmed lets it go straight to editing.
3. **Use `cargo check -p <changed-crate>` for the inner fix-iteration loop; save `cargo build
--workspace` / `cargo test --workspace` / `cargo clippy --workspace --all-targets` for one
   final pass.** Running the full triad after every edit during iteration multiplies compiler
   output round-tripped into context — `cargo check` on just the touched crate is enough to drive
   iteration, since the workspace is 15+ crates and the full suite is verbose even when green.
4. **Don't delegate what's genuinely small.** Anything under ~15 minutes of direct work (a doc
   section, a targeted fix) should be done inline rather than spun into a fork — the fork's own
   startup/re-exploration overhead can cost more than just doing it.
