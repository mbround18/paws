# AGENTS.md

## Purpose

This repository is the `paws` Rust workspace: a Dagger-backed, run-anywhere CI/CD tool. Agents should optimize for correctness, safety, and minimal scope.

## Required conventions

- Prefer surgical, reviewable changes over broad refactors.
- Keep the Rust workspace warning-free. Treat `cargo clippy --workspace --all-targets -- -D warnings` as the default gate.
- Do not leave dead code, commented-out code, or TODOs without a clear reason.
- Prefer explicit opt-in behavior for publishing or destructive actions; do not silently upload, push, or mutate external systems.
- Never log secrets, tokens, usernames, passwords, or environment values in plaintext.
- Preserve existing architecture and crate boundaries. New logic should live in the relevant crate instead of being bolted onto a random entry point.
- Keep APIs and workflow contracts backward-compatible unless the change is intentionally breaking.

## Build and validation

- During iteration, use `cargo check -p <changed-crate>` or the smallest relevant test target.
- Before finishing work, run the repo’s standard quality gate:
  - `cargo test --workspace --all-targets --quiet`
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  - `make quality`
- If a change affects release/publish flows, validate the relevant targeted tests or fixture coverage before claiming completion.

## Repo-specific rules

- Follow the repository’s existing Dagger-first design. Container execution should go through the Dagger paths, not ad hoc shelling out.
- Keep workflows safe-by-default; release, publish, or registry actions should require explicit inputs or env configuration.
- Maintain compatibility with the existing CI workflow and release contract.
- For mod-upload or environment-driven flows, prefer GitHub Environment secrets and explicit validation-before-upload behavior.

## Agent workflow

1. Read the smallest relevant files and symbols first.
2. Implement the minimum viable fix or feature.
3. Validate with targeted tests/checks.
4. Only then report completion and include what was verified.

## References

- `CONTRIBUTING.md`
- `.github/copilot-instructions.md`
- `.github/instructions/*.instructions.md`
