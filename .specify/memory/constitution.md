<!--
Sync Impact Report
- Version change: [TEMPLATE] → 1.0.0 (initial ratification)
- Modified principles: n/a (first formal instance; all five slots newly defined)
- Added sections: Core Principles (I–V), Technical Constraints, Development Workflow, Governance
- Removed sections: none
- Templates requiring follow-up: none — this is the first ratified constitution, so no
  dependent template/spec has been checked against it yet. Future `/speckit-plan` runs for
  `001-paws-core-cli` should re-run their Constitution Check against this file.
- Deferred placeholders: none
-->

# paws Constitution

## Core Principles

### I. One Crate Per Domain
Every pipeline domain (`semver`, `audit`, `docker`, `provision`, `ci`, `docs`, ...) lives in
its own crate under `crates/`, not folded into `paws-cli` or a shared god-crate. `paws-cli`
is a thin `clap`-based narrative layer that wires subcommands to domain crates; it MUST NOT
contain domain logic itself. Each crate must be small enough to unit-test in isolation and to
migrate independently (see Principle III). This mirrors `gh-reusable`'s own package split and
keeps a future `dagger-sdk` migration scoped to one crate at a time instead of a rewrite.

### II. Subprocess-First Dagger Access, Single Call Site
Dagger is invoked exclusively as a subprocess (`Command::new("dagger")`) through
`paws-dagger::call`; no other crate spawns `dagger` directly. This is enforced by a grep-based
CI lint (SC-004), not just convention. Rationale: Dagger's own Rust SDK is marked experimental
and explicitly "not for anything mission-critical" — depending on it directly would stake the
whole rewrite's stability on an unsupported dependency, the exact risk this project exists to
avoid on the GitHub Actions side.

### III. Incremental SDK Adoption
When the Rust `dagger-sdk` becomes trustworthy enough to adopt, it is piloted one crate at a
time — starting with the lowest-stakes candidate (e.g. `paws-semver`) — never adopted
workspace-wide in a single change. A crate only moves off subprocess-based `paws-dagger`
access after it has its own passing parity test suite under the new access path.

### IV. Parity Testing Over Reimplementation-From-Memory
`paws` is a port of `gh-reusable`'s existing, working pipeline logic, not a reinvention of it.
Every ported behavior MUST name the exact `gh-reusable` source file/function it asserts parity
against (e.g. "matches `actions/semver/src/tag.js`'s tagless-repo default"). When source
behavior is ambiguous or undocumented in a README, the implementer reads the actual TypeScript
source before writing the port and encodes the resolved behavior as a fixture test — the
ambiguity is resolved once, not re-discovered by every future reader.

### V. Reliability & Testability First (NON-NEGOTIABLE)
Every crate carries unit tests from day one; no crate merges with `(unimplemented)` stubs
left in a subcommand's handler path once that subcommand is documented as shipped. Contract
changes (new flags, new output shapes, changed defaults) MUST be paired with tests in the same
PR. `cargo test --workspace` MUST pass on every commit and is a required CI check (SC-002).

## Technical Constraints

- **No secrets on the command line.** No CLI flag ever carries a secret value; secrets are
  read from environment variables only. This applies to every subcommand, present and future.
- **Shared defaults live in one place.** Runtime baseline values (registries, default
  toolchain versions, tag prefixes, etc.) come from `paws-core::PipelineDefaults`, never
  hardcoded per-crate. A second crate hardcoding a value that belongs in `paws-core` is a bug,
  not a style preference.
- **No swallowed concurrent failures.** Any crate that orchestrates concurrent work (currently
  `paws-provision`'s `JoinSet`-based ecosystem installers) MUST surface every task's outcome in
  its aggregated result. An early return on first failure that hides other tasks' outcomes is a
  correctness bug (FR-014), not an acceptable simplification.
- **No undeclared cross-task dependencies.** No concurrently-orchestrated task may await
  another task's result unless that data dependency is documented in the relevant plan.md
  (FR-016). Undocumented ordering dependencies defeat the point of concurrent orchestration.

## Development Workflow

- Treat every shipped subcommand's flags as a contract from the moment it merges — this is
  pre-1.0, but backward compatibility is still the default; declare breaking changes
  explicitly rather than drifting silently.
- Pair contract changes with tests in the same PR — a flag, output shape, or default change
  without an accompanying test update does not merge.
- Keep `README.md`'s subcommand list and each feature's `tasks.md` checklist in sync as work
  lands; a shipped subcommand that isn't reflected in both is considered incomplete.
- A CI job greps for `Command::new("dagger")` outside `crates/paws-dagger` and fails the build
  if found (SC-004), enforcing Principle II mechanically rather than relying on review alone.

## Governance

This constitution supersedes informal practice notes (e.g. prior `README.md` "Principles"
bullets) wherever the two conflict; `README.md` should be updated to point here rather than
restate principles independently.

**Amendment procedure**: amendments are proposed via `/speckit-constitution`, which drafts the
updated text, computes the version bump, and prepends a Sync Impact Report to this file
documenting what changed and why. A human reviews and merges the amendment like any other PR.

**Versioning policy** (semantic versioning applied to governance):
- **MAJOR** — backward-incompatible principle removal or redefinition.
- **MINOR** — a new principle or materially expanded section added.
- **PATCH** — wording clarifications and non-semantic fixes.

**Compliance review**: every feature's `plan.md` MUST include a Constitution Check section
that verifies the plan against the principles above before implementation begins; any
identified conflict must be resolved or explicitly justified in that plan before work starts.

**Version**: 1.0.0 | **Ratified**: 2026-08-18 | **Last Amended**: 2026-08-18
