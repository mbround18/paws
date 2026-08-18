# Implementation Plan: Paws Core CLI — Rust-native Reimplementation of gh-reusable's Pipeline Surface

## Inputs

- Spec path: `specs/001-paws-core-cli/spec.md`
- Affected contracts/files (new, this repo): `Cargo.toml` (workspace), `crates/paws-cli/*`, `crates/paws-core/*`, `crates/paws-dagger/*`, `crates/paws-provision/*` (new, see Workstream 5.5)
- Reference/inspiration (read-only, external): `../gh-reusable/packages/dagger-module/src/index.ts` (`@func()` surface), `../gh-reusable/actions/{semver,docker-facts,ensure-repository}/README.md`, `../gh-reusable/specs/002-reusable-rust-pipeline/` (this repo's copy under `specs/002-reusable-rust-pipeline/`)

## Constitution Check

`.specify/memory/constitution.md` v1.0.0 is now ratified. This plan's design decisions (subprocess-first Dagger access, one crate per domain, parity testing, reliability/testability-first) are consistent with Principles I–V; no conflicts identified.

## Design Decisions

- **Dagger access stays subprocess-based, not `dagger-sdk`.** Decided in the prior conversation and restated in the spec's Assumptions: the Rust SDK is explicitly marked experimental by Dagger. Alternative considered — depend on `dagger-sdk` from day one for tighter typing — rejected because it stakes the whole rewrite's stability on an unsupported dependency, which is the exact tech-debt risk this project exists to avoid on the GitHub-Actions side.
- **One crate per domain, not one god-crate.** Mirrors `gh-reusable`'s own split (dagger-module vs. dagger-pipelines vs. per-action packages) but keeps each crate small enough to unit-test in isolation and to pilot the SDK migration on independently.
- **Parity tests over reimplementation-from-README.** `docker-facts`'s README doesn't fully document multi-service compose resolution (FR-012); plan requires reading `actions/docker-facts`'s TypeScript source directly rather than guessing from docs, and encoding the result as a fixture test so the behavior is captured once and doesn't need re-discovery.
- **Migration order**: `semver` → `audit` → `docker` (facts+release) → `provision` → `ci`. Smallest/best-documented contract first (semver), matching the pilot-crate principle already stated in `README.md`. `provision` lands before `ci` because `ci` depends on it internally (FR-015).
- **Concurrency lives in one crate (`paws-provision`), not scattered per-subcommand.** Every ecosystem's setup task is spawned through a single `JoinSet`-based orchestrator; domain crates (`paws-semver`, `paws-docker`, etc.) never spawn their own `tokio` tasks for toolchain setup — they call `paws-provision` and get an aggregated result. This keeps FR-014's "no swallowed failures" guarantee enforceable in one place instead of needing to be re-verified in every crate that happens to need more than one toolchain.

## Workstreams

1. **Workspace scaffolding hardening** — already done (Cargo workspace, `paws-cli`/`paws-core`/`paws-dagger` stubs, `cargo test --workspace` green). Remaining: wire `paws-cli`'s subcommand handlers to actually call `paws-dagger` instead of printing `(unimplemented)`.
2. **`paws semver` (User Story 2, P1)** — new `crates/paws-semver` crate. Port label-inference + last-tag lookup logic from `actions/semver`'s implementation. Resolve FR-011 (tagless-repo default) against the actual TS fallback before writing tests, not after.
3. **`paws audit` (User Story 4, P2)** — new `crates/paws-audit` crate. Port `AuditSummary`/`AuditScannerResult` shapes from `packages/dagger-module/src/audit-types.ts` and `audit-logic.ts` (referenced in the gh-reusable graph's `Dagger Audit Logic` community).
4. **`paws docker` (User Story 3, P2)** — new `crates/paws-docker` crate. Port `docker-facts` tag/context/push-gating logic; requires the e2e Docker fixture harness from FR-007 before it can be marked done.
5. **`paws provision` (User Story 5, P2)** — new `crates/paws-provision` crate. `JoinSet`-based concurrent orchestrator over per-ecosystem setup tasks (rust/rustup, node/pnpm, python/uv first; go/ruby/java/terraform/pulumi later, matching `setupGo`/`setupRuby`/`setupJava`/`setupTerraform`/`setupPulumi`). Each task still shells out to the real installer (rustup, etc.) — this crate's job is orchestration and aggregated reporting (FR-013/FR-014), not reimplementing installers.
6. **`paws ci` (User Story 1, P1)** — wires Node and Rust toolchain execution through `paws-dagger`; depends on workstream 1's handler wiring being complete, and on workstream 5 for FR-015 (concurrent provisioning inside `ci`).
7. **`paws docs` (User Story 6, P3)** — lowest priority; can slot in whenever, does not block the others.
8. **Governance/tests** — the grep-based lint for SC-004 (no `Command::new("dagger")` outside `paws-dagger`), CI wiring for Node/Rust/Docker matrix (FR-008), and a timing-based check for SC-005 (concurrent provisioning is actually faster than sequential, not just structured as tasks that happen to run one at a time).
9. **Docs/examples** — keep `README.md` subcommand list in sync as each one ships; update `specs/001-paws-core-cli/quickstart.md` (to be created) once `semver` is done, as the first usable example.

## Contract-Safety Checklist

- [ ] Every new subcommand's flags are documented in `--help` output and in this plan before merge
- [ ] `paws-dagger::call` remains the only call site that spawns `dagger`
- [ ] Runtime baseline values come from `paws-core::PipelineDefaults`, not hardcoded per-crate
- [ ] No CLI flag ever carries a secret value (env-var only, per spec's Security section)
- [ ] Each parity test names the exact `gh-reusable` source file/function it's asserting parity against
- [ ] No ecosystem's `paws-provision` task awaits another ecosystem's task unless a real data dependency is documented in this plan (FR-016)
- [ ] `paws-provision`'s aggregated result type surfaces every ecosystem's outcome, never just the first failure (FR-014)

## Validation Matrix

| Surface                          | Validation                                                                                   |
| --------------------------------- | ---------------------------------------------------------------------------------------------- |
| `paws-dagger` process wrapper     | Unit tests (arg-building) + one live integration test gated on `dagger` being on `PATH`        |
| `paws semver` parity              | Fixture-based test comparing output against `actions/semver`'s documented behavior             |
| `paws audit` parity               | Fixture-based test comparing summary shape against `AuditSummary`/`AuditScannerResult`         |
| `paws docker` parity              | Fixture `docker-compose.yml` + e2e test against a real/test-double Docker daemon (FR-007)       |
| `paws ci` (Node + Rust)           | Real fixture projects exercised in `paws`'s own CI (FR-008), not unit-mocked only              |
| No direct `dagger` shell-outs     | Grep-based CI lint (SC-004)                                                                    |
| Workspace-wide correctness        | `cargo test --workspace` on every commit (SC-002)                                              |
| `paws provision` concurrency      | Fixture with 3 mock installers of known duration; assert wall-clock ≈ max(durations), not sum (SC-005) |
| `paws provision` failure isolation | Fixture where one of three mock installers fails; assert the other two still report success (FR-014) |

## Open Questions

- ~~FR-011: confirm `actions/semver`'s actual default base version for a tagless repo.~~ **Resolved** — see spec.md FR-011: `{prefix}0.0.0`, default prefix `v`, read directly from `actions/semver/src/tag.js`.
- ~~FR-012: extract and document `docker-facts`'s multi-service `docker-compose.yml` resolution rule.~~ **Resolved** — see spec.md FR-012: first `image:`-matching service wins, read directly from `packages/dagger-pipelines/src/docker-parity.ts`.
- ~~No `paws`-native CI pipeline exists yet.~~ **Resolved**: `.github/workflows/ci.yaml` runs plain `cargo build`/`cargo test --workspace`/`cargo clippy`/the SC-004 lint on GitHub Actions. Dogfooding `paws ci` itself is deferred until task 6 (native toolchain execution, currently still interim-`dagger`-wired) ships.
- ~~`paws-provision`'s per-ecosystem task signature isn't decided yet.~~ **Resolved during scaffolding**: an `Installer` trait with a blanket impl over `Fn() -> Future<Output = Result<()>>`, boxed as `Box<dyn Installer>`, orchestrated via `JoinSet`. See `crates/paws-provision/src/lib.rs`. Real installers (rustup, pnpm/corepack, `uv`) still need to be plugged in behind this trait — that's workstream 5's remaining work, not a design question anymore.
