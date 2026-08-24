# Feature Specification: Close Remaining Gaps Found Migrating `valheim-docker`

**Feature Branch**: `005-close-remaining-cli`

**Created**: 2026-08-23

**Status**: Draft

**Input**: User description: "While migrating `mbround18/valheim-docker`'s GitHub Actions CI to `paws` (specs 003/004 closed the Docker tag-matrix and changelog gaps), four smaller gaps surfaced that weren't blocking but are worth closing: (1) `paws docs --help` claims it publishes to GitHub Pages but `run_docs` only runs `cargo doc` locally — no publish path exists; (2) `paws ci --toolchain rust`'s non-wasm `cargo clippy` step has no `-D warnings` gate, unlike the wasm path, so a new clippy warning doesn't fail CI; (3) `paws docker` has no way to reuse Dagger's build cache across separate CI runs on ephemeral GitHub-hosted runners, unlike the `cache-from`/`cache-to` registry cache the pre-migration workflow used; (4) `paws audit` runs Semgrep and Gitleaks only — no Rust dependency-vulnerability scanner (`cargo-audit`/`cargo-deny`), despite already having a `LanguageFamily::Rust` detection signal a new scanner could gate on."

## Clarifications

### Session 2026-08-23

- Q: Should the Dagger build-cache reuse (Gap 3) work by passing through `DAGGER_CLOUD_TOKEN`, or by wrapping the local engine's storage in a GitHub Actions cache? → A: Both, unified under a provider abstraction — mirroring `003-release-parity-docker`'s `HistoryProvider` precedent (a trait with multiple implementations, auto-selected by environment, no new CLI flag). This spec ships two `CacheBackend` implementations: `DAGGER_CLOUD_TOKEN` passthrough (selected when the token is present) and a GitHub Actions cache wrapping the local engine's storage (selected otherwise, when running under GitHub Actions). No cache mechanism available/detected falls through to today's no-cache behavior (FR-007), unchanged.
- Q: Should `paws docs --publish`'s underlying mechanism (Gap 1) be Git Trees API or the GitHub Pages deployment API, and should this be a `PublishTarget` provider concept too? → A: Yes to the provider concept, but shaped differently from `HistoryProvider`/`CacheBackend`: destination isn't an environment fact (both GitHub mechanisms are always available), so it's an explicit `--provider <name>` flag (mirrors `paws publish --target rust-crate`'s existing convention), not auto-selected. This spec ships `--provider github-pages` only (renaming/replacing the bare `--publish` flag) — `--provider cloudflare-pages` and `--provider s3` are recognized CLI values that fail with a clear "not implemented yet, see docs/ROADMAP.md" error, not silently ignored, tracked as follow-ups (mirroring `004-rust-coverage`'s "ship Rust, roadmap the rest" precedent). Within the `github-pages` provider itself, the Git-Trees-vs-Pages-deployment-API choice is auto-selected by querying the target repo's actual GitHub Pages configuration (`GET /repos/{owner}/{repo}/pages`'s `build_type`: `"legacy"` → Git Trees API, `"workflow"` → Pages deployment API), falling back to the Git Trees API (matching `helm --publish`/`llms generate --publish`'s existing branch-commit pattern) when Pages isn't configured yet, so a repo's first-ever publish bootstraps the same way those two already do.
- Q: When `--provider` names multiple targets and one fails, should `paws docs` report every provider's outcome before exiting non-zero, or stop at the first failure? → A: Report every outcome. `--provider` is comma-delimited (mirrors `--registries`/`--targets`'s existing `value_delimiter = ','` convention) and accepts multiple providers in one run; the `cargo doc` tree is built once and reused across all requested providers, each runs concurrently (matching `paws-provision`'s existing multi-ecosystem-installer shape), and the command exits non-zero if any provider failed while still reporting every provider's individual success/failure — the constitution's existing "no swallowed concurrent failures" technical constraint applies here, not a new failure-handling shape invented for this feature.

## Summary

Four independent, non-blocking gaps surfaced while spec'ing and dogfooding a real `paws` CI migration for `mbround18/valheim-docker` (see `003-release-parity-docker`, `004-rust-coverage`), each verified directly against this repo's current source rather than assumed:

1. **`paws docs` doesn't publish anything.** `DocsArgs` is an empty struct (`crates/paws-cli-core/src/lib.rs:675`, `pub struct DocsArgs {}`) and `run_docs` (`crates/paws-cli-core/src/lib.rs:1606-1611`) calls `paws_docs::build_docs` and prints the local `target/doc` path — nothing more. But `paws docs --help`'s own description says "Publish generated docs (e.g. rustdoc) to GitHub Pages," and `001-paws-core-cli`'s own User Story 6 describes the intended behavior as parity with `rustDocsBuild`/`rust-docs-publish.yaml`, which *does* publish. The CLI's help text is currently a promise the code doesn't keep.
2. **`paws ci --toolchain rust`'s clippy gate is inconsistent.** `crates/paws-rust/src/lib.rs`'s wasm branch runs `cargo clippy --target wasm32-unknown-unknown -- -D warnings` (lines ~148-154); the non-wasm (default) branch runs bare `cargo clippy` with no `-- -D warnings` (line ~160). A new clippy warning on the common, non-wasm path silently doesn't fail the pipeline, while the same warning class on a wasm target would.
3. **No Dagger build-cache reuse across CI runs.** Confirmed by reading `crates/paws-dagger/src/lib.rs` and `paws-up`'s composite action (`actions/paws-up/action.yml`): every subcommand shells out to a local `dagger` CLI talking to a per-runner `dagger-engine-vN` Docker container with no persisted state and no `DAGGER_CLOUD_TOKEN` (or any other remote-cache) wiring anywhere in the workspace (`grep -rn DAGGER_CLOUD_TOKEN` across `crates/`, `actions/`, `docs/`, `.github/` returns nothing). On GitHub-hosted (ephemeral) runners, this means every `paws docker`/`paws ci` invocation rebuilds from zero cache, unlike the `cache-from`/`cache-to` registry-cache pattern the pre-migration `docker/build-push-action`-based workflow relied on.
4. **`paws audit` has no Rust dependency-vulnerability scanner.** `ScannerName` (`crates/paws-audit/src/lib.rs:159-162`) is `{ Semgrep, Gitleaks }` only — both SAST/secrets tools, neither a dependency/SCA (software composition analysis) scanner. The catalog already has everything a third scanner needs: `LanguageFamily::Rust` detection (gated on `Cargo.toml`/`Cargo.lock` signals, `crates/paws-audit/src/lib.rs:238`) and a `ScannerConfig { family: ScannerFamily::Language(family), applies_to, should_run, step_name, image }` shape (`crates/paws-audit/src/lib.rs:174-180`) that Semgrep already uses per-language. There is no `cargo-audit`/`cargo-deny` (or equivalent RustSec advisory-database) scanner wired into that catalog.

None of these four block the `valheim-docker` migration itself (specs 003/004 already closed the blocking gaps) — this spec exists to track and close the smaller ones found along the way before they're forgotten.

## Motivation and Problem Statement

- **Gap 1 (docs) is a documentation-integrity problem**: a CLI whose own `--help` output doesn't match its behavior undermines the trust `llms.txt`/`--help`-driven discovery (this project's own primary UX model for both humans and AI agents) depends on.
- **Gap 2 (clippy gate) is a silent quality-gate hole**: `paws ci --toolchain rust` exists specifically to be a CI gate; a code path within it that doesn't actually gate on warnings is a correctness bug in the gate itself, not a missing feature.
- **Gap 3 (build cache) is a cost/speed problem, not correctness**: every ephemeral-runner `paws docker`/`paws ci` build pays a full rebuild every time, which the pre-migration registry-cache setup avoided. As more repos migrate to `paws`, this scales linearly with adoption.
- **Gap 4 (dependency scanner) is a coverage gap**: `paws audit`'s stated purpose is "run the audit/compliance scanner suite," but a Rust consumer today gets zero supply-chain/dependency-vulnerability coverage from it — only SAST and secrets.

## Scope

### In scope

- Implementing a `PublishTarget` provider abstraction for `paws docs` (see Clarifications, Session 2026-08-23) selected via an explicit, comma-delimited `--provider <name>[,<name>...]` flag (mirroring both `paws publish --target rust-crate`'s naming convention and `--registries`/`--targets`'s existing comma-delimited shape) — multiple providers run concurrently against one `cargo doc` build, with every provider's outcome (success or failure) reported and the command exiting non-zero if any failed, matching `paws-provision`'s existing "no swallowed concurrent failures" pattern — this spec ships the `github-pages` provider only, reusing the existing Contents-API/Git-based publish machinery already proven for `paws helm --publish` and `paws llms generate --publish` where the mechanism transfers, and explicitly resolving where it doesn't (see Edge Cases — a `cargo doc` output tree is hundreds of files, not the single `index.yaml`/`llms.txt` file those two publish today). Within the `github-pages` provider, the underlying write mechanism (Git Trees API vs. GitHub's Pages deployment API) is itself auto-selected by querying the target repo's actual Pages configuration.
- Recognizing `--provider cloudflare-pages` and `--provider s3` as valid CLI values that fail with a clear, actionable "not implemented yet" error (not a silent no-op and not an "unrecognized value" parse error) — reserving the `PublishTarget` surface for those providers without building them in this spec.
- Adding `-- -D warnings` to `paws-rust`'s non-wasm clippy invocation, matching the wasm branch's existing gate exactly.
- Adding a build-cache-reuse path to `paws docker`/`paws ci`'s Dagger invocations via a `CacheBackend` provider abstraction (see Clarifications, Session 2026-08-23) with two shipped implementations: `DAGGER_CLOUD_TOKEN` passthrough (Dagger's own native remote cache) and a GitHub Actions cache wrapping the local engine's persistent storage — auto-selected by environment, same pattern `003-release-parity-docker`'s `HistoryProvider` already established.
- Adding a Rust dependency-vulnerability scanner (`cargo-audit` or `cargo-deny`, decided in plan.md) to `paws audit`'s existing scanner catalog, gated on the existing `LanguageFamily::Rust` detection signal.

### Out of scope

- Actually implementing the `cloudflare-pages` and `s3` `PublishTarget` providers — this spec only reserves their CLI surface (`--provider cloudflare-pages`/`--provider s3` recognized, clear "not implemented" error) and tracks them in `docs/ROADMAP.md`, matching `004-rust-coverage`'s "ship one, roadmap the rest" precedent. Each is real follow-up work: Cloudflare Pages needs a Cloudflare API token and its Direct Upload API; S3 needs AWS credentials and a completely different upload/auth path than either GitHub mechanism.
- Any non-Rust coverage/scanner tooling (Node/Python/Go dependency scanners) — this spec's scanner addition is Rust-only, matching `004-rust-coverage`'s precedent of scoping a new capability to one toolchain first.
- Redesigning `paws-audit`'s detection/catalog architecture — the existing `ScannerName`/`ScannerConfig`/`LanguageFamily` shapes are reused as-is, not changed.
- A general-purpose remote-cache abstraction for non-Docker `paws ci` toolchains (Node/Python/Go build caching) — this spec's cache work is scoped to what `paws docker`/Dagger itself already supports natively.
- Retroactively fixing every other subcommand's `--help` text against its actual behavior — Gap 1 fixes `paws docs` specifically because it was found and verified; a general help-text audit is separate, future work.

## Affected Contracts

- **`paws docs` CLI contract**: `DocsArgs` gains new fields — `--provider <github-pages|cloudflare-pages|s3>[,...]` (Clarifications, Session 2026-08-23; comma-delimited, no default, required when publishing), `--repository`, `--branch` (mirroring `HelmArgs`'/`LlmsArgs`' existing `--publish` shape, but keyed off `--provider`'s presence instead of a separate `--publish` boolean) — additive, default-off, so `paws docs` with no flags keeps today's local-build-only behavior. `--provider cloudflare-pages`/`--provider s3` are valid values that fail fast with a "not implemented yet" error rather than a clap parse error or a silent no-op. Multiple providers run concurrently against one `cargo doc` build; every provider's outcome is reported, and the command exits non-zero if any failed (Clarifications, Session 2026-08-23) — matching `paws-provision`'s existing aggregated-outcome contract, not a new shape.
- **`paws-rust` contract**: no CLI-visible change — `cargo clippy`'s exact invocation gains `-- -D warnings` on the non-wasm path. This is a **behavior-visible, not signature-visible** change: existing green pipelines with an existing, previously-ignored clippy warning will start failing. See Risks and Rollout.
- **`paws docker`/`paws ci` Dagger invocation contract**: a new `CacheBackend` provider abstraction (Clarifications, Session 2026-08-23) — env-var passthrough and Actions-cache-wrapping implementations, auto-selected by environment — is additive; no cache backend detected behaves exactly as today (full rebuild), so this is backward compatible by construction. No new CLI flag: selection is automatic, matching `HistoryProvider`'s established no-flag precedent.
- **`paws audit` contract**: `AuditScannerResult`'s existing shape (used by every current consumer of audit output) gains one more possible `scanner` value; no existing field or scanner's behavior changes.

## Runtime and Defaults Impact

- Docs publish (Gap 1) needs a GitHub token for the same reason `helm --publish`/`llms generate --publish` do — reuses the existing token-resolution path (`$GH_APP_CLIENT_ID`/`$GH_APP_PRIVATE_KEY` or a plain PAT), no new secret category.
- Build cache (Gap 3), if resolved via `DAGGER_CLOUD_TOKEN` passthrough, needs no new `paws-core::PipelineDefaults` field — it's an existing Dagger-native env var, just documented and confirmed to reach the `dagger` subprocess unmodified (`paws-dagger`'s `Command::new("dagger")` inherits the calling process's environment by default, so this may already work with zero code change — verifying that is itself part of this spec's Validation Plan, not an assumption to skip).
- The new Rust scanner (Gap 4) needs no new runtime config — it runs the same way Semgrep/Gitleaks already do, containerized via Dagger, gated by the same repository-signal detection.

## Security and Permissions Impact

- Docs publish (Gap 1): same `contents: write`-equivalent requirement `helm --publish` already has — no new permission category, just one more code path exercising it.
- Build cache (Gap 3): the `DAGGER_CLOUD_TOKEN` provider is itself a new secret a consumer opts into (Dagger Cloud account required) when present; the GitHub Actions cache provider needs only the `actions: read/write`-equivalent scope a runner already has by default for `actions/cache`-style usage, no new secret. Both are optional — no backend detected/configured leaves behavior unaffected.
- New scanner (Gap 4): a Rust dependency scanner reads `Cargo.lock`/crates.io advisory data — no write access, no new permission scope, same containerized-scanner sandboxing Semgrep/Gitleaks already use.

## Risks and Mitigations

- **Risk (Gap 2, most significant)**: turning on `-- -D warnings` on the non-wasm clippy path is a breaking default-behavior change for any existing `paws ci --toolchain rust` consumer that currently has an un-addressed clippy warning — their next CI run fails where it previously passed.
  **Mitigation**: this is a deliberate, declared breaking change (per the constitution's "declare breaking changes explicitly rather than drifting silently"), not something to soften with an opt-out flag — an opt-out would defeat the entire point of fixing a gate that doesn't gate. Document it prominently in the release notes for whatever `paws` version ships this, with the one-line local reproduction (`cargo clippy -- -D warnings`) a consumer can run before upgrading to check exposure.
- **Risk (Gap 1)**: a `cargo doc` output tree can be hundreds of files; naively looping `put_content` (the single-file Contents-API writer `helm`/`llms generate` use) per file would be slow and could hit secondary rate limits.
  **Mitigation**: FR-003 requires a bulk mechanism (Git Trees API single-commit tree write, or the Pages deployment/artifact API, auto-selected per the target repo's actual Pages configuration) rather than assuming `put_content` scales to a directory tree.
- **Risk (Gap 1, provider surface)**: shipping `--provider cloudflare-pages`/`--provider s3` as recognized-but-unimplemented values risks a consumer assuming they work (since the flag is accepted) until they actually try one.
  **Mitigation**: FR-004a requires the error for those two values to be immediate and explicit ("not implemented yet — see docs/ROADMAP.md"), not deferred until deep into a publish attempt, and `docs/ROADMAP.md` names both as tracked follow-ups rather than silently-absent capability.
- **Risk (Gap 1, multi-provider)**: running multiple providers concurrently against a shared `cargo doc` build tree risks one provider's failure being masked by another's success (or vice versa) if outcomes aren't tracked independently — the exact class of bug the constitution's "no swallowed concurrent failures" constraint already exists to prevent for `paws-provision`.
  **Mitigation**: FR-002a requires every named provider's outcome to be individually tracked and reported in the aggregated result, reusing `paws-provision`'s already-proven pattern rather than a new, unproven concurrent-aggregation implementation.
- **Risk (Gap 3)**: either `CacheBackend` provider silently does nothing if misconfigured (e.g. `DAGGER_CLOUD_TOKEN` set but not actually reaching the `dagger` subprocess; the Actions-cache provider running outside GitHub Actions where its API isn't available) — a consumer could set things up and see zero cache benefit with no error.
  **Mitigation**: FR-006 requires a verification test (or explicit runtime log line confirming which backend, if any, was selected) so "I configured caching and nothing happened" is diagnosable for either provider, not a silent no-op.
- **Risk (Gap 3, provider selection)**: with two backends auto-selected by environment (Clarifications, Session 2026-08-23), an ambiguous environment (e.g. both `DAGGER_CLOUD_TOKEN` set *and* running under GitHub Actions) needs a defined precedence, or selection could be nondeterministic across runs.
  **Mitigation**: FR-005 requires a documented, fixed precedence order between the two providers (resolved in plan.md, e.g. `DAGGER_CLOUD_TOKEN` wins when present, since it's the more explicit opt-in signal) — not left to whichever check happens to run first in the code.
- **Risk (Gap 4)**: a new dependency scanner surfaces real, possibly-noisy findings in every Rust consumer's very first `paws audit` run post-upgrade (e.g. a known-but-accepted RustSec advisory in a transitive dependency).
  **Mitigation**: match Semgrep/Gitleaks's existing pattern of reporting findings without failing the build by default (confirm this is in fact their current behavior before assuming it — see Validation Plan) so the new scanner's rollout doesn't unexpectedly break builds the same day it ships.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - `paws ci --toolchain rust` actually fails on a clippy warning (Priority: P1)

As a maintainer relying on `paws ci --toolchain rust` as my CI gate, I want a new clippy warning to fail the build on the default (non-wasm) path, the same way it already does for a wasm target, so warnings don't silently accumulate past a gate that's supposed to catch them.

**Why this priority**: this is the only one of the four gaps that's a correctness bug in an existing, shipped gate rather than a missing capability — it directly undermines what `paws ci` already claims to do today.

**Independent Test**: run `paws ci --toolchain rust` against a fixture crate with one deliberate, otherwise-harmless clippy warning (e.g. a `#[allow]`-free `.clone()` clippy flags); assert it now exits non-zero, where before this fix it exited 0.

**Acceptance Scenarios**:

1. **Given** a non-wasm Rust fixture with a clippy warning and no `#![deny(warnings)]` in its own source, **When** `paws ci --toolchain rust` runs, **Then** the pipeline fails at the clippy step with that warning surfaced in output.
2. **Given** the same fixture with the warning fixed, **When** `paws ci --toolchain rust` runs, **Then** it passes exactly as before this change (no behavior change for a clean crate).
3. **Given** a wasm fixture (unaffected by this change), **When** `paws ci --toolchain rust` runs, **Then** its clippy gating behavior is unchanged (it already had `-- -D warnings`).

---

### User Story 2 - Dagger build cache survives across separate CI runs (Priority: P2)

As a maintainer running `paws docker`/`paws ci` on ephemeral GitHub-hosted runners, I want build layers to be reused across separate workflow runs, not just within a single run, so my Docker/Rust build times don't regress compared to the pre-migration `cache-from`/`cache-to` registry-cache setup.

**Why this priority**: real cost/speed impact that scales with adoption, but nothing is *incorrect* without it — today's behavior (full rebuild every run) is slow, not wrong.

**Independent Test**: run `paws docker` twice in a row against the same unchanged Dockerfile/context, in two genuinely separate processes/environments (simulating two separate ephemeral runners, not just two calls in the same shell session) with each `CacheBackend` provider configured in turn; assert the second run's wall-clock time is materially lower than the first, attributable to reused layers, not local-filesystem reuse.

**Acceptance Scenarios**:

1. **Given** no `CacheBackend` provider's environment signature matches (no `DAGGER_CLOUD_TOKEN`, not running under GitHub Actions), **When** `paws docker` runs twice in separate environments, **Then** behavior is unchanged from today — both runs pay a full build (no regression from adding this feature).
2. **Given** `DAGGER_CLOUD_TOKEN` is set, **When** `paws docker` runs twice in separate environments with an unchanged Dockerfile, **Then** the `DAGGER_CLOUD_TOKEN` provider is selected and the second run's build step reuses cached layers and completes materially faster.
3. **Given** `DAGGER_CLOUD_TOKEN` is unset but the run is under GitHub Actions, **When** `paws docker` runs twice across separate jobs with an unchanged Dockerfile, **Then** the GitHub Actions cache provider is selected and the second run reuses cached layers.
4. **Given** both `DAGGER_CLOUD_TOKEN` is set *and* the run is under GitHub Actions, **When** `paws docker` runs, **Then** the documented fixed precedence (Risks: `DAGGER_CLOUD_TOKEN` wins) decides which single provider is used — never both, never nondeterministically.
5. **Given** either provider is configured but a build step's inputs actually changed (e.g. `Cargo.lock` changed), **When** the second run executes, **Then** only the invalidated layers rebuild — the mechanism must respect normal Docker/Dagger cache-invalidation semantics, not force a blanket full-cache-hit.

---

### User Story 3 - `paws audit` flags a known-vulnerable Rust dependency (Priority: P2)

As a maintainer running `paws audit` on a Rust project, I want it to also check my dependency tree against the RustSec advisory database (via `cargo-audit` or `cargo-deny`), so a known-vulnerable transitive dependency shows up in the same audit summary Semgrep/Gitleaks findings already do, instead of requiring a separate manual tool.

**Why this priority**: real coverage gap, but — unlike Gap 2 — nothing today claims to already cover this; it's a capability addition, not a broken promise.

**Independent Test**: run `paws audit` against a fixture Rust project with a `Cargo.lock` pinning a package version with a known, published RustSec advisory; assert the finding appears in `AuditSummary`'s output with the same shape (status, findings, confidence ranking) Semgrep/Gitleaks findings already use.

**Acceptance Scenarios**:

1. **Given** a Rust fixture with no known-vulnerable dependencies, **When** `paws audit` runs, **Then** the new scanner reports zero findings and does not affect the overall audit outcome.
2. **Given** a Rust fixture with a `Cargo.lock` pinning a package with a known RustSec advisory, **When** `paws audit` runs, **Then** the finding appears in the summary with the same fields (`scanner`, severity/confidence, description) `AuditScannerResult` already defines for Semgrep findings.
3. **Given** a non-Rust project (no `Cargo.toml`/`Cargo.lock` signal), **When** `paws audit` runs, **Then** the new scanner's `should_run` resolves false via the existing `LanguageFamily::Rust` detection — it is skipped, not run and vacuously passed.

---

### User Story 4 - `paws docs --provider github-pages` actually publishes to GitHub Pages (Priority: P3)

As a maintainer of a Rust project with a public API surface, I want `paws docs --provider github-pages` to publish the generated `cargo doc` output to GitHub Pages, matching what its own `--help` text already claims, so I don't have to hand-roll a separate publish step.

**Why this priority**: lowest priority of the four — it's a real gap, but the immediate motivating case (`valheim-docker`) has no public API surface (odin/huginn are internal binaries), so nothing in that migration depends on this landing. Still worth fixing because the CLI's own documentation currently overpromises.

**Independent Test**: run `paws docs --provider github-pages` against a fixture Rust workspace with doc comments; assert the generated `target/doc` tree is published to the target Pages branch and is retrievable afterward (e.g. via the same Contents/Git API used to publish it), and that a second identical run is idempotent (no-op or a no-diff commit, not a duplicate/conflicting publish).

**Acceptance Scenarios**:

1. **Given** `--provider` is omitted, **When** `paws docs` runs, **Then** behavior is byte-identical to today — local `cargo doc` build only, nothing published (no regression from adding this flag).
2. **Given** `--provider github-pages` is set with valid credentials, **When** `paws docs` runs, **Then** the full `target/doc` tree is published to the configured Pages branch/deployment (per FR-003's auto-selected mechanism) in a way that survives a directory with hundreds of files (see Risks — not a naive per-file Contents-API loop).
3. **Given** `--provider github-pages` is set but the GitHub token lacks write access, **When** `paws docs` runs, **Then** it fails with a specific, actionable error before or during publish — not a silent partial publish.
4. **Given** `--provider cloudflare-pages` or `--provider s3` is set, **When** `paws docs` runs, **Then** it fails immediately with the FR-004a "not implemented yet" error — no build/publish attempt, no partial work.
5. **Given** `--provider github-pages,s3` is set (one implemented, one not), **When** `paws docs` runs, **Then** the `cargo doc` tree is built once, `github-pages` succeeds and publishes normally, `s3` fails with the FR-004a error, both outcomes are reported, and the command exits non-zero — the `github-pages` success is never hidden by the `s3` failure, and vice versa (Clarifications, Session 2026-08-23).

### Edge Cases

- What happens when `paws docs --provider github-pages` is run against a workspace whose `cargo doc` output hasn't changed since the last publish? → Must be a safe no-op (or a no-diff commit), not an error and not a duplicate publish — same idempotency bar `llms generate --publish` already holds itself to ("Skips the commit if the generated content is unchanged").
- What happens when `--provider` is given a value other than `github-pages`/`cloudflare-pages`/`s3`? → A normal clap "invalid value" parse error, same as any other enum-shaped flag in `paws` (e.g. `--toolchain`) — distinct from the FR-004a "not implemented yet" error, which only applies to the two valid-but-unbuilt provider names.
- What happens when the clippy `-D warnings` gate (Gap 2) is added and a consumer's crate has a warning specifically from a dependency's macro expansion, not their own code? → Out of scope to special-case; this is standard `cargo clippy -- -D warnings` behavior already familiar to any Rust maintainer who runs it locally, not a new problem this feature introduces.
- What happens when `DAGGER_CLOUD_TOKEN` (if chosen for Gap 3) is invalid/expired? → Must degrade to the current no-cache behavior with a clear warning, not fail the entire pipeline — a broken cache backend should never be worse than no cache backend.
- What happens when the new Rust dependency scanner (Gap 4) and Semgrep both flag the same underlying issue (e.g. Semgrep's Rust ruleset already flagging an unsafe-pattern that overlaps a RustSec advisory)? → No deduplication required for this spec; both findings surface independently, same as Semgrep and Gitleaks already can both fire on unrelated aspects of the same file today.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: `paws-rust`'s non-wasm build path MUST append `-- -D warnings` to its `cargo clippy` invocation, exactly matching the flag set the wasm path already uses (`cargo clippy --target wasm32-unknown-unknown -- -D warnings`), minus the wasm-specific `--target`.
- **FR-002**: `paws docs` MUST gain an opt-in, comma-delimited `--provider <name>[,<name>...]` flag (Clarifications, Session 2026-08-23; no default, omitted means no publish); with it omitted, behavior MUST remain byte-identical to today (local `cargo doc --workspace --no-deps` build only, per `paws-docs::build_docs`).
- **FR-002a**: When `--provider` names more than one target, `paws docs` MUST build the `cargo doc` output tree exactly once and reuse it across every named provider — never rebuilding per provider. Each named provider MUST run concurrently (not sequentially), matching `paws-provision`'s existing multi-ecosystem-installer concurrency shape, and every provider's outcome (success or a specific failure reason) MUST be surfaced in the aggregated result — the command exits non-zero if any provider failed, but a failure on one provider MUST NOT hide or suppress another provider's outcome (the constitution's existing "no swallowed concurrent failures" constraint, applied here rather than newly invented).
- **FR-003**: `paws docs --provider github-pages`'s underlying write mechanism MUST be auto-selected by querying the target repo's actual GitHub Pages configuration (`GET /repos/{owner}/{repo}/pages`'s `build_type`): `"legacy"` → a bulk Git Trees API commit (create tree, create commit, update ref in one pass); `"workflow"` → the GitHub Pages deployment/artifact API; no Pages configuration yet → fall back to the Git Trees API. Never a naive per-file loop over the existing single-file `put_content` Contents-API writer, which does not scale to a `cargo doc` output tree's file count.
- **FR-004**: `paws docs --provider github-pages` MUST accept `--repository`/`--branch` inputs mirroring `paws helm --publish`'s existing flag shape, for consistency across `paws`'s publish-capable subcommands.
- **FR-004a**: `paws docs --provider cloudflare-pages` and `--provider s3` MUST be accepted as valid CLI values (not rejected as an invalid `--provider` value) and MUST fail with an explicit, actionable "not implemented yet — see docs/ROADMAP.md" error before attempting any publish work — never a silent no-op, never treated as `github-pages`.
- **FR-005**: `paws docker`/`paws ci`'s Dagger invocation path MUST gain a `CacheBackend` provider abstraction (Clarifications, Session 2026-08-23) with two implementations — `DAGGER_CLOUD_TOKEN` passthrough and a GitHub Actions cache wrapping the local engine's storage — auto-selected by environment with a documented, fixed precedence when both providers' signatures match (`DAGGER_CLOUD_TOKEN` wins when present). No new CLI flag for selection, matching `003-release-parity-docker`'s `HistoryProvider` precedent.
- **FR-006**: Whichever `CacheBackend` provider is selected (or none) MUST be independently verifiable — a consumer must be able to confirm which backend was actually engaged (e.g. a log line naming the active backend, or an explicit "no cache backend available" line), not infer it indirectly from build speed alone.
- **FR-007**: With no `CacheBackend` provider's environment signature matched, `paws docker`/`paws ci` behavior MUST remain byte-identical to today — this is an additive, opt-in-by-environment capability, not a default-behavior change (unlike FR-001, which is a deliberate default change).
- **FR-008**: `paws audit` MUST add one new scanner (`cargo-audit` or `cargo-deny`, decided in plan.md) to its existing `ScannerName`/`ScannerConfig` catalog, gated on the existing `LanguageFamily::Rust` detection signal (`crates/paws-audit/src/lib.rs`'s `detect_with_signals` for `Cargo.toml`/`Cargo.lock`).
- **FR-009**: The new Rust dependency scanner's findings MUST be parsed into the existing `AuditScannerResult`/`TopFinding` shapes Semgrep findings already use (per `parse_semgrep_findings`'s pattern) — no separate/divergent output shape for this one scanner.
- **FR-010**: The new scanner MUST NOT run (and MUST NOT vacuously "pass") for a project with no `Cargo.toml`/`Cargo.lock` signal — `should_run` resolves false exactly as any other non-applicable `LanguageFamily`-gated scanner already does.
- **FR-011**: Each of the four gaps in this spec MUST ship with unit tests per Constitution Principle V — none merges with an `(unimplemented)` stub in its subcommand's handler path.

### Key Entities

- **`PublishTarget` Provider**: the abstraction (FR-002–FR-004a, Clarifications Session 2026-08-23) behind `paws docs --provider <name>[,...]`, selected explicitly via a comma-delimited CLI flag (not auto-selected by environment, unlike `HistoryProvider`/`CacheBackend` — destination is a deliberate choice, not an environment fact). This spec ships `github-pages` only; `cloudflare-pages`/`s3` are reserved values that fail fast with a clear error. Multiple named providers run concurrently against one `cargo doc` build, with every provider's outcome aggregated and reported (FR-002a) — the same shape `paws-provision::Ecosystem`'s multi-target concurrent installers already use.
- **Docs Publish Target**: the `(repository, branch, path-tree)` triple the `github-pages` provider resolves and writes to — analogous to `paws helm --publish`'s `(repository, pages_branch, index_path)` but for a whole directory tree instead of one file.
- **Clippy Gate**: the exact `cargo clippy [-- -D warnings]` invocation `paws-rust` constructs per target (wasm vs. non-wasm) — after this spec, both branches carry the same warnings-as-errors gate.
- **Cache Backend Provider**: the abstraction (FR-005–FR-007, Clarifications Session 2026-08-23) behind which build-cache reuse runs, mirroring `003-release-parity-docker`'s `HistoryProvider` shape. Two implementations ship in this spec — `DAGGER_CLOUD_TOKEN` passthrough and a GitHub Actions cache wrapper — auto-selected by environment with a fixed precedence (`DAGGER_CLOUD_TOKEN` wins when both match); `paws docker`/`paws ci`'s Dagger invocation either uses the selected provider or gracefully falls through to no-cache behavior.
- **Rust Dependency Scanner**: the new `ScannerConfig` catalog entry for `cargo-audit`/`cargo-deny`, gated on `LanguageFamily::Rust`, alongside Semgrep and Gitleaks in the existing catalog.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 100% of `paws ci --toolchain rust` runs against a non-wasm fixture with a real clippy warning fail at the clippy step — zero false negatives (the entire point of FR-001).
- **SC-002**: 100% of `paws ci --toolchain rust` runs against a clean, warning-free fixture continue to pass — zero false positives introduced by FR-001.
- **SC-003**: A `paws docs --provider github-pages` run against a multi-hundred-file `cargo doc` output tree completes without hitting a GitHub API secondary rate limit in normal (non-adversarial) usage.
- **SC-004**: A second `paws docker` build, against an unchanged Dockerfile/context, in a genuinely separate environment from the first, completes in materially less wall-clock time than the first, under each `CacheBackend` provider configured in turn (`DAGGER_CLOUD_TOKEN` and the GitHub Actions cache wrapper) — the specific percentage target for each is set in plan.md, since the two mechanisms are expected to have different characteristics worth measuring independently rather than a single blended target.
- **SC-005**: `paws audit` against a fixture with a known RustSec-advisory dependency surfaces that finding in 100% of runs, with zero false negatives on that specific fixture.

## Assumptions

- **Resolved** (see Clarifications, Session 2026-08-23): `paws docs` gains a `--provider <name>` flag (a `PublishTarget` abstraction, explicit not auto-selected) with `github-pages` shipped in this spec and `cloudflare-pages`/`s3` reserved-but-unimplemented. Within `github-pages`, the Git-Trees-vs-Pages-deployment-API choice is auto-selected by querying the target repo's actual Pages configuration (`build_type`), falling back to Git Trees API when unconfigured.
- **Resolved** (see Clarifications, Session 2026-08-23): Gap 3 ships both `DAGGER_CLOUD_TOKEN` passthrough and a GitHub Actions cache wrapper as `CacheBackend` provider implementations, auto-selected by environment (mirroring `003-release-parity-docker`'s `HistoryProvider` abstraction) rather than choosing one — plan.md resolves the precedence mechanics and the Actions-cache wrapper's concrete implementation (e.g. `actions/cache`-equivalent semantics against the local Dagger engine's persistent storage path), not which one to build.
- Constitution Principle IV's parity-testing bar does not apply cleanly to any of these four gaps — none has a `gh-reusable` TypeScript source to assert parity against (clippy gating is a `paws`-native default; Dagger caching, a Rust dependency scanner, and a docs-tree publish mechanism are all new capabilities `gh-reusable` never had in this exact shape). Each is treated as new, `paws`-native behavior, documented as such rather than a claimed port.
- FR-001 (the clippy fix) is intentionally not gated behind a flag — the risk of breaking an existing consumer's CI is accepted and documented (see Risks and Mitigations) rather than deferred behind opt-in, which would leave the gate broken by default indefinitely.
- The new Rust dependency scanner (Gap 4) follows Semgrep/Gitleaks's existing default of reporting findings without failing the build outright — this assumption is verified, not just assumed, per the Risks and Mitigations note on this gap; if Semgrep/Gitleaks actually do fail builds on findings today, FR-008's new scanner follows that same behavior instead for consistency.

## Validation Plan

- Unit tests in `paws-rust` asserting the non-wasm clippy invocation now includes `-- -D warnings`, plus a fixture-based integration test proving a real warning now fails the pipeline (User Story 1's acceptance scenarios).
- Unit tests in `paws-docs` for: `--provider` omitted (no behavior change), `--provider github-pages` with a multi-file tree against both FR-003 mechanisms (Git Trees API and Pages deployment API, exercised via the Pages-config query), idempotent re-publish of unchanged content, a clear failure on insufficient token permissions, `--provider cloudflare-pages`/`--provider s3` each producing the FR-004a "not implemented yet" error immediately, and a multi-provider case (`--provider github-pages,s3`) asserting the `cargo doc` tree is built exactly once, both outcomes are independently reported, and the command exits non-zero without suppressing the successful provider's result (FR-002a).
- Confirm, before implementing FR-005, whether `Command::new("dagger")` in `paws-dagger` already inherits `DAGGER_CLOUD_TOKEN` from the calling process's environment with zero code changes (likely, since `tokio::process::Command` inherits the parent environment by default) — this determines whether FR-005/FR-006 is primarily a documentation-and-verification task or requires new plumbing code.
- A real two-separate-environment timing comparison (not just two calls in one shell session, which could benefit from local filesystem/Docker-layer reuse unrelated to the chosen remote-cache mechanism) validating SC-004.
- Unit tests in `paws-audit` for: the new scanner's `ScannerConfig` entry, its `should_run` gating on `LanguageFamily::Rust` (both present and absent), and its findings-parsing function against a fixture `cargo-audit`/`cargo-deny` JSON report containing a known advisory ID.
- `cargo test --workspace` continues to pass with zero regressions in any existing `paws-rust`, `paws-docs`, `paws-docker`, or `paws-audit` test.

## Rollout and Rollback

- FR-001 (clippy gate) ships as a default-behavior change in a clearly-labeled `paws` release, with the local reproduction command called out in that release's notes so consumers can check their own exposure before upgrading — this is the one gap in this spec where "roll out to everyone immediately" is correct (see Risks and Mitigations).
- FR-002 through FR-004a (docs publish) and FR-005 through FR-007 (build cache) ship as opt-in — zero behavior change for any consumer who doesn't pass the new flag/configure the new cache mechanism, so no coordinated rollout is required.
- FR-008 through FR-010 (Rust dependency scanner) ships as an additive scanner in the existing `paws audit` catalog — every current `paws audit` consumer with a Rust project sees new findings appear on their very next run with no flag needed (matching how Semgrep/Gitleaks already apply automatically once their language/signal is detected); if this proves too noisy in practice, the mitigation is the same one Semgrep/Gitleaks already rely on (report, don't fail, by default), not a new opt-out flag.
- If any of the four regresses, the rollback path is per-gap: revert the specific `paws-rust`/`paws-docs`/`paws-docker`/`paws-audit` change; none of the four's rollback affects the others, since they're independent user stories with no cross-dependency.
