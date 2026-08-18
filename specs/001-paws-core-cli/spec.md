# Feature Specification: Paws Core CLI — Rust-native Reimplementation of gh-reusable's Pipeline Surface

**Feature Branch**: `001-paws-core-cli`

**Created**: 2026-08-18

**Status**: Draft

**Input**: User description: "Rework gh-reusable's TypeScript Dagger pipelines and composite actions into a single Rust CLI (`paws`), organized as a Cargo workspace under `crates/*`, with subcommands mirroring the existing action/module surface. Prioritize reliability and testability across Node, Rust, Docker, and end-to-end targets. Not tied to GitHub Actions — the same binary must run in CI or on a laptop."

## Summary

`gh-reusable` today delivers its pipeline behavior as: (a) a TypeScript Dagger module (`packages/dagger-module`, class `GhReusablePipelines`) exposing ~28 `@func()` entrypoints, (b) a thin orchestration package (`packages/dagger-pipelines`) with its own CLI (`node dist/cli.js ci|build-and-push|workflow`), (c) a set of standalone composite GitHub Actions (`actions/semver`, `actions/docker-facts`, `actions/ensure-repository`, `actions/github-catalog`, `actions/graphql`, `actions/install-cli`, `actions/setup-rust`), and (d) a vendored reporting bundle (`.github/actions/dagger-report`). All of it is invoked exclusively from GitHub Actions workflow YAML.

This feature replaces that surface with `paws`: a single, statically-linked Rust binary (`crates/paws-cli`, produced via `clap`) that exposes the same capabilities as first-class subcommands, calls into Dagger through `crates/paws-dagger` (CLI-subprocess-backed today, not the experimental `dagger-sdk` crate), and is runnable identically from a shell, from GitHub Actions, or from any other CI system.

## Motivation and Problem Statement

- **Portability**: today's pipelines are only invokable through GitHub Actions YAML (`uses: mbround18/gh-reusable/actions/...`). GitHub outages block all pipeline execution, including local reproduction of a failing check. A single binary removes that dependency.
- **Fragmentation**: pipeline logic is split across a Dagger module, a separate pipelines package, and several standalone TS actions with their own `package.json`/`tsconfig`/build steps — each with independent test setup and dependency trees. This raises the ramp-time cost documented in the gh-reusable knowledge graph (`graphify-out/GRAPH_REPORT.md`): the largest communities in that graph are vendored bundle internals, not hand-written logic, because `.github/actions/dagger-report` ships a bundled `undici`/fetch dependency tree alongside real code.
- **SDK risk containment**: the Rust `dagger-sdk` crate is explicitly marked experimental by its maintainers ("do not use it for anything mission-critical"). This spec deliberately does not depend on it yet — see Assumptions.
- **Sequential toolchain setup is wasted wall-clock time**: `gh-reusable`'s current `setupRust`, `setupNode`, `setupGo`, `setupRuby`, `setupJava`, `setupTerraform`, `setupPulumi`, and Python/`uv` provisioning are each independent `@func()` entrypoints with no shared state and no dependency on one another — nothing about installing Rust requires pnpm to exist first, or vice versa. Today they only run concurrently if a workflow author happens to fan them out into parallel GitHub Actions jobs; `paws` should make "run every independent ecosystem setup step at once" the default, not something the caller has to remember to arrange.

## Concurrency Model

`paws` is built on [`tokio`](https://docs.rs/tokio/latest/tokio/) and treats independent, unrelated work as parallel by default rather than opt-in:

- Any step that talks to a different package ecosystem (Rust/cargo, Node/pnpm, Python/uv, Go, Ruby, Java, Terraform, Pulumi) MUST be modeled as an independent `tokio` task with no implicit ordering dependency on another ecosystem's setup, so "prefetch Rust and pnpm toolchains at the same time" is the normal code path, not a special case.
- Concurrency is scoped to genuinely independent work only — steps with a real data dependency (e.g. `docker build` needing `docker-facts`' resolved tags first) stay sequential. This is about eliminating *accidental* serialization, not forcing parallelism where a real dependency exists.
- Failure in one concurrent branch MUST NOT silently swallow or mask a failure in another — every branch runs to completion and every error surfaces in the aggregated result, not just the first one encountered (FR-014).

## Scope

### In scope

- A Cargo workspace (`Cargo.toml` at repo root, `crates/*` members) producing one binary: `paws`.
- `crates/paws-cli`: `clap`-derived subcommands covering the current `GhReusablePipelines` `@func()` surface (see Key Entities), each independently testable.
- `crates/paws-core`: shared contract types — the Rust equivalent of `PipelineDefaults`/`defaults.json`, docker-facts-style config shapes, and the semver increment model.
- `crates/paws-dagger`: process wrapper around the `dagger` CLI (`dagger call -m <module> <function> ...`), the single seam through which all pipeline execution flows.
- Reliability/testability requirements: unit tests per crate, a Docker-based e2e harness for the docker-facts/docker-build path, and CI that runs on Node, Rust, and Docker targets identically to how `gh-reusable`'s own `test-*` workflows validate parity today.
- Migration mapping from every current `@func()` and composite action to a named `paws` subcommand (this spec + plan enumerate that mapping; implementation is tracked in `tasks.md`).
- A `paws-provision` crate that fans out independent toolchain setup (Rust, pnpm/Node, uv/Python, and later Go/Ruby/Java/Terraform/Pulumi) as concurrent `tokio` tasks, with aggregated success/failure reporting across all of them.

### Out of scope

- Depending on the Rust `dagger-sdk` crate for actual Dagger calls (deferred — see Assumptions and `paws-dagger`'s doc comment).
- Rewriting `gh-reusable` itself. `gh-reusable`'s existing TS implementation remains the reference/behavioral source of truth during migration and is not modified by this feature.
- A GUI, web dashboard, or hosted service — `paws` is a CLI only.
- Publishing/release automation for `paws` itself (crates.io, binary releases) — tracked as a future feature once the core surface is stable.

## Affected Contracts

- **New CLI contract**: `paws <subcommand> [--flags]` — see Key Entities for the full subcommand list. Each subcommand's flags must be a documented, versioned, backward-compatible-by-default contract, mirroring how `gh-reusable` treats reusable-workflow inputs and `@func()` signatures as public contracts.
- **`paws-dagger` contract**: `DaggerCall { module, function, args }` → `Result<String>`. Every higher-level crate calls through this, never `dagger` directly.
- **No existing gh-reusable contracts change.** This is a net-new consumer of the same conceptual behavior, not a modification of `gh-reusable`'s reusable workflows, actions, or Dagger module.

## Runtime and Defaults Impact

- `paws-core` must define a `PipelineDefaults`-equivalent struct (already stubbed) that plays the same role as `gh-reusable`'s `defaults.json`/`DEFAULTS_PATH` — a single runtime baseline (toolchain versions, registries) consumed by every subcommand rather than hardcoded per-command.
- Container/runtime baseline: `paws` itself must build and run without a Node toolchain present. Any subcommand that shells into Dagger still requires the `dagger` CLI on `PATH` (documented, checked with a clear error, not a silent failure — see FR-010).

## Security and Permissions Impact

- `paws-dagger` must never construct shell strings from user input (use `tokio::process::Command` argument vectors, not a shell) to avoid injection when subcommand flags are interpolated into `dagger call` invocations.
- Secrets (e.g. `CARGO_REGISTRY_TOKEN`, `DOCKER_TOKEN` equivalents) must be read from environment variables at the point of use, never accepted as CLI flags, matching `gh-reusable`'s existing secret-handling convention in its reusable workflow contracts.
- No new GitHub permissions are required by this feature — `paws` does not talk to the GitHub API directly.

## Risks and Mitigations

- **Risk**: Behavioral drift between the TS implementation (still live in `gh-reusable`) and the Rust reimplementation.
  **Mitigation**: Each ported subcommand's acceptance scenario (below) requires a parity test asserting identical output/exit-code behavior against a fixture, not just "it runs."
- **Risk**: `dagger` CLI absence breaks every subcommand identically without a clear signal.
  **Mitigation**: `paws-dagger::call` must produce a distinct, actionable error (already implemented) rather than a generic process-spawn failure; FR-010 makes this testable.
- **Risk**: Rust `dagger-sdk` immaturity tempts an early full migration.
  **Mitigation**: explicitly out of scope until a pilot crate (semver, per `README.md`) has run in production for a defined trial period.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Run CI checks without GitHub (Priority: P1)

As a maintainer, I want to run the same build/lint/test pipeline `paws` runs in CI, from my laptop with no network dependency on GitHub, so that a GitHub outage never blocks me from validating a change.

**Why this priority**: this is the entire reason `paws` exists instead of continuing to extend the TS Dagger module in place.

**Independent Test**: run `paws ci --toolchain node` (and separately `--toolchain rust`) against a real checkout with GitHub Actions network access disabled; both complete with the same pass/fail result as the equivalent `gh-reusable` workflow run.

**Acceptance Scenarios**:

1. **Given** a Node project fixture with a failing lint rule, **When** `paws ci --toolchain node` runs, **Then** it exits non-zero and reports the specific lint failure, matching `pnpmBuildAndTest`'s current failure reporting.
2. **Given** a Rust project fixture that builds and tests cleanly, **When** `paws ci --toolchain rust` runs, **Then** it exits 0 and the same command produces an identical result when run inside GitHub Actions.

---

### User Story 2 - Compute and apply semantic version bumps (Priority: P1)

As a maintainer, I want `paws semver` to compute the next version from PR labels or an explicit increment, matching `actions/semver`'s current behavior, so downstream release automation doesn't need to change.

**Why this priority**: semver is the smallest, best-isolated existing contract (see `actions/semver/README.md`) and is the designated pilot crate for evaluating the Rust `dagger-sdk` later — it has to be correct and fully ported first.

**Independent Test**: run `paws semver --branch main` against a repo fixture with a known last tag and a known label set; compare output to `actions/semver`'s documented `new_version` output for the same fixture.

**Acceptance Scenarios**:

1. **Given** no `base` is supplied, **When** `paws semver` runs, **Then** it looks up the last tag the same way `actions/semver` does today and increments it according to `major-label`/`minor-label`/`patch-label` matches.
2. **Given** an explicit `--increment` is supplied, **When** `paws semver` runs, **Then** label inference is skipped entirely, matching the current action's documented precedence.

---

### User Story 3 - Build and gate a Docker image the same way docker-facts does (Priority: P2)

As a maintainer, I want `paws docker` to derive Dockerfile path, build context, tags, and push-gating from `docker-compose.yml` (or fallback inputs) exactly as `actions/docker-facts` + `docker-build` do today, so container releases don't regress during migration.

**Why this priority**: docker-facts is the most complex existing contract (multi-registry tags, canary-label push gating, multi-stage target support) and is the best test of whether the Rust CLI can hold parity on a genuinely non-trivial existing behavior.

**Independent Test**: run `paws docker --push` and `paws docker` (no push) against a fixture repo with a `docker-compose.yml`; assert the resolved `dockerfile`, `context`, `tags`, and `push` decision match `docker-facts`'s documented outputs for the same fixture, including the `canary_label` and `force_push` gating rules.

**Acceptance Scenarios**:

1. **Given** a `docker-compose.yml` defines the Dockerfile/context, **When** `paws docker` runs without `--push`, **Then** `push` resolves to `false` unless `force_push` or a matching canary label is present, matching current gating logic.
2. **Given** no `docker-compose.yml` exists, **When** `paws docker` runs, **Then** it falls back to `./Dockerfile` and `.` exactly as `docker-facts`'s documented defaults do.

---

### User Story 4 - Run the audit/compliance scanner suite (Priority: P2)

As a maintainer, I want `paws audit` to aggregate scanner results (the current `audit()` `@func()` in `dagger-module`) into the same summary shape downstream tooling already consumes, so the audit/compliance pipeline spec work already done for `gh-reusable` isn't wasted.

**Why this priority**: `gh-reusable` already has a dedicated spec for this (`specs/001-enforce-dagger-compliance/` — see the copy under this repo's `specs/002-reusable-rust-pipeline/` inspiration set's sibling context); the audit contract shape is well-documented and a good second parity test after semver.

**Independent Test**: run `paws audit` against a fixture repo with a known scanner finding; assert the summary shape (status, findings, confidence ranking) matches the existing `AuditSummary`/`AuditScannerResult` shapes.

**Acceptance Scenarios**:

1. **Given** a fixture with no findings, **When** `paws audit` runs, **Then** it exits 0 with an empty findings summary.
2. **Given** a fixture with a known scanner finding, **When** `paws audit` runs, **Then** the finding appears in the summary with the same fields `AuditScannerResult` currently produces.

---

### User Story 5 - Prefetch independent toolchains concurrently (Priority: P2)

As a maintainer, I want `paws ci` (and any subcommand that needs more than one ecosystem, e.g. a repo with both a Rust crate and a pnpm workspace) to install Rust, Node/pnpm, and Python/uv toolchains at the same time rather than one after another, so I get the reliability of `gh-reusable`'s existing per-language setup functions without paying for their setup time sequentially.

**Why this priority**: this is a cross-cutting capability other stories (`ci` in particular) depend on for their *performance* characteristics, but no individual toolchain setup is more correct running in parallel than in sequence — it's a speed/architecture requirement, not a correctness-blocking one, hence P2 rather than P1.

**Independent Test**: run `paws provision --toolchains rust,node,python` against a fixture with no toolchains pre-installed; assert wall-clock time is close to the slowest single toolchain's install time, not the sum of all three, and that each toolchain ends up correctly installed regardless of the others' outcome.

**Acceptance Scenarios**:

1. **Given** Rust, Node, and Python toolchains are all requested, **When** `paws provision` runs, **Then** all three installs start concurrently (verifiable via timestamps in `--verbose` output) and none blocks on another's completion.
2. **Given** the Node toolchain install fails (e.g. network error) while Rust and Python succeed, **When** `paws provision` completes, **Then** it reports the Node failure clearly alongside the two successes — a failure in one ecosystem must not be silently absorbed by, or block reporting on, the others.
3. **Given** `paws ci` is run against a repo containing both a Rust crate and a pnpm workspace, **When** it provisions toolchains before running build/test, **Then** it internally uses the same concurrent provisioning as `paws provision`, not a sequential loop.

---

### User Story 6 - Publish generated docs (Priority: P3)

As a maintainer, I want `paws docs` to publish generated documentation (starting with `paws`'s own `cargo doc` output) to GitHub Pages the same way `rustDocsBuild`/`rust-docs-publish.yaml` do today for other Rust crates in `gh-reusable`.

**Why this priority**: lowest risk, most deferrable — docs publishing has no correctness-critical downstream consumer, unlike semver or docker tags.

**Independent Test**: run `paws docs` against `paws`'s own workspace; assert a `target/doc` (or equivalent) artifact is produced and the command is idempotent (safe to re-run).

**Acceptance Scenarios**:

1. **Given** a workspace with doc comments, **When** `paws docs` runs, **Then** it produces a docs artifact without requiring network access beyond what `cargo doc` itself needs.

---

### Edge Cases

- What happens when the `dagger` CLI is not installed or not on `PATH`? → Must fail fast with a specific, actionable error (see FR-010), not a generic "command not found."
- What happens when `paws semver` is run in a repo with no tags at all? → Returns `{prefix}0.0.0` (default prefix `v`) per FR-011, not a panic.
- What happens when `paws docker` is run against a `docker-compose.yml` with multiple services? → Selects the first service whose `image:` matches `{imageName}:`; no match falls back to the non-compose defaults rather than guessing — per FR-012.
- What happens when a subcommand is run outside a Cargo/Node/Docker project entirely (no recognizable project markers)? → Must produce a clear "nothing to do here" message, not attempt partial work.
- What happens when `paws provision` is asked for a toolchain that isn't installable in the current environment (e.g. `--toolchains rust,node` but the sandbox has no network access)? → Both are still attempted concurrently; the unreachable one fails with a specific network/timeout error, the other succeeds and is reported as such (FR-014) — one bad ecosystem never blocks visibility into the rest.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: `paws` MUST expose one subcommand per migrated capability (`ci`, `semver`, `docker`, `audit`, `docs`, with more added as later `@func()` entries are ported), each documented via `clap`'s built-in `--help`.
- **FR-002**: Every subcommand MUST route Dagger execution exclusively through `paws-dagger::call`, never spawn `dagger` (or any other process) directly from `paws-cli` or a domain crate.
- **FR-003**: `paws semver` MUST reproduce `actions/semver`'s label-inference precedence: explicit `--increment` overrides label inference; label inference checks `major-label` before `minor-label` before `patch-label`.
- **FR-004**: `paws docker` MUST reproduce `docker-facts`'s tag generation and push-gating logic, including `canary_label` and `force_push` semantics, before this feature can be considered complete for User Story 3.
- **FR-005**: `paws-core` MUST provide a single `PipelineDefaults`-equivalent type consumed by every subcommand that needs a runtime baseline, so toolchain/registry defaults are defined once, not per-subcommand.
- **FR-006**: Every crate (`paws-cli`, `paws-core`, `paws-dagger`, and each ported domain crate) MUST ship unit tests exercising its public API, with `cargo test --workspace` as the single command that runs all of them.
- **FR-007**: The docker-facts parity path (User Story 3) MUST have an end-to-end test that runs against a real `docker-compose.yml` fixture and a real (or test-double) Docker daemon, not just unit-level mocking, since tag/push logic is the highest-risk parity surface.
- **FR-008**: CI for `paws` itself MUST validate the Node, Rust, and Docker targets it claims to support — i.e., `paws ci --toolchain node` and `paws ci --toolchain rust` must each be exercised against a real fixture project in `paws`'s own CI, not just unit-tested in isolation.
- **FR-009**: No subcommand MAY depend on the `dagger-sdk` Rust crate; all Dagger interaction goes through the CLI-subprocess wrapper until the pilot (User Story 2, semver) explicitly evaluates the SDK.
- **FR-010**: If the `dagger` CLI is missing from `PATH`, `paws` MUST report a specific error naming the missing binary and a remediation hint, not a raw OS-level "No such file or directory."
- **FR-011**: `paws semver` MUST handle a tagless repository by returning `{prefix}0.0.0` as the last tag, where `prefix` defaults to `"v"` when not explicitly provided — confirmed from `actions/semver/src/tag.js:71-74` (`getLastTag`'s no-tags-found branch). Additionally, when tags exist but none match the resolved prefix, the same `{prefix}0.0.0` fallback applies (`tag.js:118-124`). Full resolved precedence for `paws semver` to reproduce exactly:
  1. If running on a tag ref (`GITHUB_REF` starts with `refs/tags/`), return that tag verbatim — no increment applied (`tag.js:37-42`, `version.js:26-40`).
  2. If an explicit `--increment` is supplied, it wins outright — no label or branch inference runs (`increment.js:183-186`).
  3. Otherwise, for a PR event: check the PR's labels for `major-label`/`minor-label`/`patch-label` matches (major beats minor beats patch, `increment.js:59-67`); if no configured label is present, fall back to branch-name pattern matching against `BRANCH_INCREMENT_RULES` (`major`/`breaking` → major; `feat`/`feature`/`minor`/`release/*` → minor; `fix`/`patch`/`hotfix`/`bugfix`/`chore`/`docs`/`refactor`/`test`/`ci`/`perf` → patch, `increment.js:4-22`); if nothing matches, default to `patch`.
  4. For a push event, look up the PR associated with the pushed commit via its SHA and apply the same label→branch→patch fallback chain to that PR's labels.
  5. PR builds produce a prerelease version `{prefix}{incremented}-pr.{shortSha}` (7-char SHA) rather than a plain increment (`version.js:60-70`).
  6. Prefix inference: if no `prefix` is given and every existing tag starts with `v`, infer `v` as the prefix; the resolver also considers a hyphenated variant of the prefix (e.g. `v-`) and picks whichever prefix produces the larger set of valid-semver-matching tags (`tag.js:76-116`).
- **FR-012**: `paws docker`'s compose resolution, extracted from `packages/dagger-pipelines/src/docker-parity.ts` (`findDockerCompose`/`parseDockerCompose`, lines 226-302), MUST reproduce this exact behavior:
  1. Compose file discovery checks, in order, `docker-compose.yml`, `docker-compose.yaml`, `compose.yml`, `compose.yaml` in the workspace root first, then (if not found there) in the resolved `context` directory.
  2. When a compose file has multiple `services:` entries, iterate them in file order and select the **first** service whose `image:` value starts with `{imageName}:` — there is no "first service wins" fallback when none match; an unmatched image name yields empty dockerfile/context/target/buildArgs (the non-compose fallback path), not an error and not an arbitrary service pick.
  3. A matched service's `build:` field may be a bare string (treated as `context` only) or a record with `dockerfile`/`context`/`target`/`args` — each is mapped through as-is; missing fields stay `null`/empty rather than inheriting a default.
  4. This selection logic MUST be captured as a fixture test with at least two services in one compose file (one matching, one not) before User Story 3 is considered ported — a single-service fixture does not exercise FR-012's actual risk (silent wrong-service selection).
- **FR-013**: `paws provision` MUST launch each requested toolchain's setup (Rust, Node/pnpm, Python/uv, and any later-added ecosystem) as an independent `tokio` task (e.g. via a `JoinSet`), with no task awaiting another task's completion unless a real data dependency exists between them.
- **FR-014**: `paws provision` MUST run every requested toolchain's setup to completion and aggregate all results — one ecosystem's install failure MUST be reported alongside every other ecosystem's outcome (success or failure), never swallowed by an early return on first error.
- **FR-015**: `paws ci` MUST use the same concurrent provisioning path as `paws provision` internally whenever a target repo requires more than one ecosystem (e.g. a repo with both `Cargo.toml` and a pnpm workspace) — provisioning must not regress to a sequential loop inside `ci` even though `provision` exists as its own subcommand.
- **FR-016**: Concurrency introduced for provisioning MUST NOT be applied to steps with a genuine ordering dependency (e.g. `docker` needing docker-facts' resolved tags before `docker build` runs) — the concurrency model (see `## Concurrency Model`) applies only where `gh-reusable`'s current `@func()` surface already treats the steps as independent.

### Key Entities

- **Subcommand surface** (source of truth: `packages/dagger-module/src/index.ts` `@func()` methods on `GhReusablePipelines`): `ci`, `pnpmBuildAndTest`, `enforcePrLabels`, `notifyDiscord`, `rustBuildAndTest`, `rustDocsBuild`, `rustPipeline`, `audit`, `publishNpm`, `publishPnpm`, `publishYarn`, `publishRustCrate`, `publishHelmChart`, `dockerRelease`, `ensureRepository`, `graphqlQuery`, `installCli`, `setupRust`, `pythonBuildAndTest`, `setupNode`, `setupGo`, `setupRuby`, `setupJava`, `setupTerraform`, `setupPulumi`, `rustBinaryRelease`, `computeSemver`, `dockerFacts`, `codeMatrix`. This spec ports the P1/P2 subset (`ci`, `computeSemver`→`semver`, `dockerFacts`+`dockerRelease`→`docker`, `audit`); the remainder is tracked for follow-up features once this core surface is proven.
- **`PipelineDefaults`**: the Rust equivalent of `gh-reusable`'s `defaults.json`-backed runtime baseline (toolchain, registry). Already stubbed in `paws-core`.
- **`DaggerCall`**: `{ module, function, args }` — the one shape every subcommand constructs to reach Dagger. Already implemented in `paws-dagger`.
- **Toolchain Provisioner**: the concurrent-setup abstraction in `paws-provision` — takes a set of requested ecosystems (rust, node, python, ...), launches one `tokio` task per ecosystem, and returns an aggregated `{ecosystem: Result<...>}` map so every outcome is visible regardless of others' success/failure.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: `paws ci`, `paws semver`, `paws docker`, and `paws audit` each produce output matching their `gh-reusable` TS equivalents on the same fixture inputs, verified by an automated parity test per subcommand.
- **SC-002**: `cargo test --workspace` passes with zero failures on every commit to `paws`'s default branch (enforced by `paws`'s own CI once bootstrapped).
- **SC-003**: A contributor with no prior context can run `paws --help` and correctly identify which subcommand replaces a given `gh-reusable` action/function within 2 minutes, without reading source.
- **SC-004**: Zero direct `Command::new("dagger")` (or any shell-out) call sites exist outside `crates/paws-dagger` — enforced by a grep-based lint in CI (mirrors `gh-reusable`'s own governance-test pattern of verifying wiring via static checks, e.g. `workflow-governance.test.ts`).
- **SC-005**: `paws provision --toolchains rust,node,python` against a fixture with none pre-installed completes in wall-clock time within 20% of the single slowest toolchain's standalone install time (not the sum of all three), and every ecosystem's outcome — success or failure — is present in the final report.

## Assumptions

- The Rust `dagger-sdk` crate remains out of scope for this spec's entire lifetime; a future spec will revisit it once a pilot (semver) has run for a defined trial period, per the project `README.md`'s stated principle.
- `dagger` CLI availability on the developer's/CI's `PATH` is a precondition, not something `paws` installs itself, for this iteration.
- `gh-reusable`'s TS implementations remain the behavioral source of truth for parity testing until `paws` fully replaces a given subcommand, at which point `gh-reusable` itself may deprecate the corresponding action (tracked separately, out of scope here).
- `tokio` (multi-thread runtime, already a workspace dependency) is the fixed async runtime for all of `paws`; no subcommand introduces a second async runtime or blocks the runtime with synchronous I/O for work that could be a task.
- Toolchain installers invoked by `paws-provision` (rustup, a Node/pnpm installer, `uv`) are still shelled out to as external processes for this iteration — `paws-provision`'s job is orchestrating them concurrently via `tokio`, not reimplementing what rustup/uv already do well.
- This spec assumes a single target platform baseline (Linux/macOS on `x86_64`/`aarch64`) for the initial CLI binary; Windows support is deferred and not a blocking requirement for User Stories 1-5.
