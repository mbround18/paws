# Copilot instructions for paws

## Mission

This repository is a Rust workspace for a Dagger-based CI/CD tool. The goal is to make the project safe, portable, testable, and easy to operate across local and GitHub-hosted environments.

## Core expectations

- Keep the codebase warning-free and clippy-clean.
- Prefer explicit, secure, opt-in behaviors for publish/release workflows.
- Do not add dead code, unused helpers, or commented-out code.
- Keep changes scoped to the relevant crate or workflow; avoid unrelated cleanup.
- Preserve the repo’s architecture and public contracts unless a breaking change is explicitly required.

## Rust workflow

- Prefer `cargo check -p <crate>` during iteration.
- Run the final validation gate before finishing:
  - `cargo test --workspace --all-targets --quiet`
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  - `make quality`
- Follow the crate layout and use targeted validation instead of broad speculative refactors.

## Security and release safety

- Treat all credentials as sensitive. Never print them or include them in logs or failures.
- Prefer environment-backed secrets and validation-before-upload patterns.
- Keep default publishing behavior disabled until a caller opts in.
- When adding external integrations, surface clear, structured errors rather than broad panic paths.

## Documentation and collaboration

- Keep docs and examples aligned with behavior changes.
- When adding workflow or CLI behavior, update the relevant docs or quickstart examples.
- Keep instructions and task descriptions concrete, domain-specific, and implementation-aware.

## Review standard

Before completing a change, confirm:

- the fix matches the actual repository conventions
- the relevant tests pass
- the output is not noisy or speculative
- the change is minimal and reviewable
