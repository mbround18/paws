# Feature Specification: Expose the GitHub Actions Cache Runtime Vars `GitHubActionsCache` Depends On

**Feature Branch**: `006-paws-doesn-expose`

**Created**: 2026-08-24

**Status**: Implemented

**Input**: User description: "While auditing `mbround18/valheim-docker`'s finished `paws` CI migration for leftover non-paws tooling ('trim the fat'), I found that `005-close-remaining-cli`'s `CacheBackend::GitHubActionsCache` reads `$ACTIONS_CACHE_URL`/`$ACTIONS_RUNTIME_TOKEN` from the process environment, but GitHub Actions does not expose those two variables to a plain `run:` shell step (or a bash-only composite action's steps, which is exactly what `paws-up` and `paws init` are) — only to a JS/Node-based action step, which reads them from the runner's internal environment and has to explicitly re-export them via `$GITHUB_ENV` for later shell steps to see. `paws-up` has no such step today. Spec closing this gap so `GitHubActionsCache` actually activates on a real GitHub-hosted runner instead of silently detecting `CacheBackend::None` every time."

## Summary

`005-close-remaining-cli` shipped `paws_dagger::CacheBackend::detect()`, which selects `GitHubActionsCache` when `$ACTIONS_CACHE_URL` and `$ACTIONS_RUNTIME_TOKEN` are both present in the process environment (`crates/paws-dagger/src/lib.rs`, `CacheBackend::detect`). This is correct as far as it goes, but it depends on those two variables actually being *in* the environment `paws docker`/`paws ci` runs in — and on a real GitHub-hosted runner, invoked the way every current `paws` consumer invokes `paws` (a `run:` step in a workflow, or a step inside `paws-up`'s bash-only composite action), they are not. GitHub Actions withholds `ACTIONS_RUNTIME_TOKEN`/`ACTIONS_CACHE_URL` (and the newer `ACTIONS_RESULTS_URL` equivalents) from a plain shell step's environment; they're only visible to a JS/Node-based action step, which reads them off the runner's internal process environment and must explicitly `core.exportVariable`/write to `$GITHUB_ENV` for a later shell step to inherit them. This is the entire reason third-party actions like `crazy-max/ghaction-github-runtime` exist, and it's the documented prerequisite step in Dagger's own GitHub Actions cache-volume guidance.

`paws-up` (`actions/paws-up/action.yml`) and `paws init` (`crates/paws-cli-core/src/lib.rs::run_init`) are both bash-only today — no JS/Node step anywhere in either. That means `CacheBackend::detect()` reliably returns `CacheBackend::None` on every current GitHub-hosted-runner invocation of `paws docker`/`paws ci`, regardless of whether the runner actually has cache-service access — `GitHubActionsCache`'s entire implementation (confirmed live-tested per `005`'s commit message, presumably in a context where those vars *were* manually exported) never activates for a real `paws-up`-based consumer. This is a silent gap: nothing errors, `CacheBackend::log_line()` (FR-006 from `005`) faithfully reports `"cache: no backend detected (full rebuild)"` — but a consumer reading `paws docker --help`/`docs/ROADMAP.md`'s description of automatic, zero-config GitHub Actions caching would reasonably expect it to just work, and today it structurally cannot.

## Motivation and Problem Statement

- **`005`'s own FR-006 ("independently verifiable, not inferred from build speed") is the thing that surfaces this gap** — without that log line, a consumer might never notice the cache backend is always `None`. With it, running `paws docker` today on a real `paws-up`-provisioned GitHub Actions runner should print `cache: no backend detected (full rebuild)` on every single run, never `cache: using github-actions`, which is itself evidence this spec exists to explain and fix.
- **The feature is unusable as shipped, not just suboptimal.** This isn't a performance tuning gap — `GitHubActionsCache` cannot activate at all today for any consumer following the documented, zero-config usage path (`paws-up` → `paws docker`/`paws ci`). A feature that can never turn on is effectively dead code from a real consumer's perspective, even though `005`'s unit tests (which presumably inject the env vars directly into the test process, not through a real runner's actual variable-scoping behavior) pass.
- **Found via real dogfooding, not speculation**: this surfaced specifically while auditing `mbround18/valheim-docker`'s finished migration (see the prior "trim the fat" conversation) for whether the new caching capability was worth wiring in — the answer was "not yet, because it can't actually turn on," which is a `paws`-side gap, not a consumer-side integration mistake.

## Scope

### In scope

- Making `$ACTIONS_CACHE_URL`/`$ACTIONS_RUNTIME_TOKEN` (or their replacement, if plan.md's Open Question resolves toward the newer results-service vars instead — see Assumptions) actually reach `paws docker`/`paws ci`'s process environment when invoked the standard way: `paws-up` on a GitHub-hosted runner, no extra consumer-side step.
- Deciding, in plan.md, the specific mechanism: (a) `paws-up` gains a JS/Node-based step (either depending on an existing, narrowly-scoped action like `crazy-max/ghaction-github-runtime`, or a `paws`-native equivalent implemented as an inline `actions/github-script` snippet or a small dedicated JS/TS action shipped from this repo); (b) some non-JS mechanism that reads the same underlying runner-internal source `ghaction-github-runtime` reads, if one is confirmed to exist (this spec does not assume JS is strictly required — see Assumptions/Open Question — only that today's bash-only `paws-up` doesn't do it).
- Re-verifying `005`'s `CacheBackend::GitHubActionsCache` end to end against a *real* GitHub Actions job using the fixed `paws-up`, not just against a test process with the vars injected directly — closing the verification gap `005`'s own tests apparently didn't catch.
- Updating `docs/ROADMAP.md`'s `005` entry and `paws docker`/`paws ci --help` text (if either currently implies zero-extra-setup activation) to accurately describe what's now required, if anything remains required after this fix.

### Out of scope

- Adding `$ACTIONS_RESULTS_URL` (the newer Twirp/protobuf results-service) support — `005` already scoped that out explicitly as a known follow-up; this spec only fixes exposure of the *existing* legacy-service vars `GitHubActionsCache` already targets, not adding support for the newer service.
- Any change to `CacheBackend::detect()`'s selection precedence (`DaggerCloud` winning when both signatures are present) — that logic is correct and untouched; this spec is purely about making its inputs actually reachable.
- Non-GitHub-Actions CI providers — `paws_environment::Provider` is GitHub-only today (per `001-paws-core-cli`), and this spec doesn't change that scope.
- Re-litigating whether `GitHubActionsCache` was the right design (`005` already made and shipped that call) — this spec fixes an activation bug in an already-accepted design, not a design review.

## Affected Contracts

- **`paws-up` composite action contract**: gains a new step (mechanism TBD in plan.md) that runs before any `paws` subcommand needing Dagger. If it depends on a third-party action, that's a new, named dependency this action didn't have before — must be pinned to a specific version/SHA, not a floating tag, matching how `paws-up` itself is meant to be consumed (`version: latest` is explicitly opt-in dogfooding language in its own doc comment; a dependency `paws-up` pulls in on a consumer's behalf should not carry that same floating-version risk).
- **`paws init` contract**: if the fix instead (or additionally) belongs in `paws init` rather than purely in the composite action, `run_init`'s behavior gains this responsibility — resolved in plan.md, not assumed here.
- **No change to `paws_dagger::CacheBackend`'s public shape** — `detect()`'s env-var names and precedence logic are unchanged; only what populates those env vars before `detect()` runs changes.

## Runtime and Defaults Impact

- No new required consumer-facing input — the entire point is that `paws-up` alone (already required for any `paws` subcommand needing Dagger) is sufficient, with no new flag or step a consumer has to remember to add.
- If the chosen mechanism depends on an external action, `paws-up`'s own effective dependency surface grows by one pinned action — document this in `paws-up`'s `action.yml` description, matching how its existing doc comments already explain *why* each of its current steps exists.

## Security and Permissions Impact

- `ACTIONS_RUNTIME_TOKEN` is a scoped, job-lifetime credential the runner itself issues — exposing it to later steps is exactly what `GitHubActionsCache`'s design in `005` already assumes happens; this spec doesn't change what has access to it, only makes the thing `005` already assumed true actually true.
- If a third-party action is chosen as the mechanism (Scope, In scope, item (a)), it must be reviewed for what else it does with the runner environment before being pinned — a runtime-token-exporting action is, by necessity, touching sensitive internals, so an unreviewed dependency here carries more risk than a typical marketplace action pin.

## Risks and Mitigations

- **Risk**: The chosen third-party action (if that's the resolved mechanism) is unmaintained or gets yanked, silently breaking every `paws-up` consumer's cache activation (though never their correctness — `CacheBackend::None` is always a safe fallback, per `005`'s FR-007).
  **Mitigation**: pin to a specific commit SHA, not a floating major-version tag; note in `paws-up`'s own doc comment (matching its existing style of explaining *why* each step exists) that this dependency exists specifically for `GitHubActionsCache` activation and what breaks (cache silently stays off, nothing else) if it stops working.
- **Risk**: `005`'s existing unit tests for `CacheBackend::detect()` inject the env vars directly into the test process and pass, giving false confidence this already works end to end — the same blind spot that let this gap ship in the first place.
  **Mitigation**: FR-004 requires a real-runner validation (not just unit tests) as part of this spec's own Validation Plan, specifically to avoid repeating `005`'s verification gap.
- **Risk**: A `paws`-native (non-third-party) implementation of the runtime-token export turns out to need a JS/Node action, which is a new category of dependency for a project whose every other action is currently pure bash (`paws-up`, and every other `actions/*/action.yml` in this repo) — this could be read as scope creep into a new tooling category.
  **Mitigation**: this exact tradeoff (reuse a narrow, well-known existing action vs. add first-ever JS-action tooling to this repo) is the Open Question left to plan.md, not decided here — see Assumptions.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - `GitHubActionsCache` actually activates on a real GitHub-hosted runner (Priority: P1)

As a maintainer running `paws docker`/`paws ci` via `paws-up` on a real GitHub-hosted runner, with no extra step of my own, I want `CacheBackend::detect()` to actually find `$ACTIONS_CACHE_URL`/`$ACTIONS_RUNTIME_TOKEN` and select `GitHubActionsCache`, so the zero-config caching `005` shipped actually turns on instead of silently staying `None` forever.

**Why this priority**: this is the only story in this spec — it's a single, focused bug fix for a feature that currently cannot activate at all through its documented, zero-config usage path.

**Independent Test**: run a real GitHub Actions workflow (not a local/simulated test process) using the fixed `paws-up`, invoke `paws docker` against a trivial Dockerfile, and read the job log for `CacheBackend::log_line()`'s output; assert it reads `"cache: using github-actions"`, not `"cache: no backend detected (full rebuild)"`.

**Acceptance Scenarios**:

1. **Given** a GitHub Actions job using the fixed `paws-up` with no extra consumer-added step, **When** `paws docker` (or `paws ci`) runs, **Then** its log output includes `"cache: using github-actions"`.
2. **Given** the same fixed `paws-up`, but running in a context genuinely outside GitHub Actions (e.g. a bare local shell, or a different CI provider), **When** `paws docker`/`paws ci` runs, **Then** it still resolves `CacheBackend::None` cleanly — the fix must not fabricate/leak stale values outside a real Actions job.
3. **Given** `$DAGGER_CLOUD_TOKEN` is also set in the same job, **When** `paws docker`/`paws ci` runs, **Then** `DaggerCloud` still wins per `005`'s existing precedence — this fix doesn't change that ordering.
4. **Given** a second `paws docker` run in a *separate* job on an unchanged Dockerfile (the real end-to-end point of `GitHubActionsCache` existing at all), **When** it runs after the fix, **Then** it completes in materially less wall-clock time than a cold run, attributable to the now-actually-restored cache — this is `005`'s own SC-004, re-verified here now that the backend can actually be reached.

### Edge Cases

- What happens if the chosen mechanism's dependency (a third-party action, if that's the resolved approach) itself fails or times out? → Must degrade to `CacheBackend::None` (a warning, not a hard failure) — a broken exposure mechanism should never be worse than the current always-`None` state, mirroring `005`'s own existing "broken cache backend is never worse than no cache backend" principle for an invalid/expired token.
- What happens on a self-hosted runner that doesn't have the legacy Cache Service v1 API available at all (regardless of variable exposure)? → Falls through to `CacheBackend::None` exactly as it does today — this spec fixes exposure of vars that exist to be exposed; it doesn't fabricate cache-service access where none exists.
- What happens if a consumer's workflow already has its own step exporting these vars (e.g. they already added `ghaction-github-runtime` themselves before this fix shipped)? → `paws-up`'s new step must not conflict with or double-export in a way that breaks — idempotent re-export (last-write-wins on `$GITHUB_ENV`, which is how `$GITHUB_ENV` already behaves) is acceptable; this is not a scenario that needs special-casing beyond "don't crash."

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: `paws-up` MUST, when running on a real GitHub Actions runner, ensure `$ACTIONS_CACHE_URL` and `$ACTIONS_RUNTIME_TOKEN` are present in the environment of every subsequent step that invokes `paws docker`/`paws ci` within that job — with no additional step required from the consumer's own workflow YAML.
- **FR-002**: The mechanism chosen to satisfy FR-001 (third-party action, `paws`-native JS/Node action, or another approach confirmed to work) MUST be decided explicitly in plan.md, including the specific tradeoff named in Risks and Mitigations (introducing this repo's first JS/Node-based action tooling vs. depending on an existing narrow third-party action).
- **FR-003**: If a third-party action is chosen, it MUST be pinned to a specific commit SHA (not a floating tag), consistent with the security posture the rest of this spec's Security and Permissions Impact section requires for a runtime-token-adjacent dependency.
- **FR-004**: This spec's fix MUST be validated against a real GitHub Actions job (per User Story 1's Independent Test), not only unit tests injecting the target env vars directly into a test process — closing the exact verification gap that let `005` ship with `GitHubActionsCache` unreachable in practice.
- **FR-005**: Outside a real GitHub Actions job (local shell, a different CI provider, or a GitHub Actions job where the legacy Cache Service v1 API genuinely isn't available), the fix MUST NOT fabricate or leak values that make `CacheBackend::detect()` incorrectly select `GitHubActionsCache` — `CacheBackend::None` must remain the correct, safe outcome in every context where the underlying cache-service access doesn't actually exist.
- **FR-006**: `docs/ROADMAP.md`'s existing `005-close-remaining-cli` entry and any `--help` text implying zero-extra-setup `GitHubActionsCache` activation MUST be corrected (or, if this spec fully closes the gap, confirmed accurate) to reflect what's actually required after this fix ships — no repeat of the original overpromise pattern `005` itself was created to fix (`paws docs --help`).
- **FR-007**: This fix MUST ship with a test proving FR-005's negative case (no false-positive detection outside a real Actions job) — not just a positive-path test proving FR-001's activation.

### Key Entities

- **Runtime Token Exposure Step**: the new `paws-up` step (mechanism resolved in plan.md) responsible for making `$ACTIONS_CACHE_URL`/`$ACTIONS_RUNTIME_TOKEN` visible to later shell steps in the same job.
- **`CacheBackend::detect()`**: unchanged from `005` — this spec is entirely about what's true in the environment by the time this function runs, not the function itself.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A real GitHub Actions job using the fixed `paws-up`, with zero extra consumer-added steps, logs `"cache: using github-actions"` on every `paws docker`/`paws ci` invocation where the legacy Cache Service v1 API is genuinely available — 100% activation rate on a real runner, where today it is 0%.
- **SC-002**: Zero false-positive `GitHubActionsCache` selections outside a real GitHub Actions job across the fix's test suite (FR-007).
- **SC-003**: A two-separate-job timing comparison (per `005`'s own SC-004, now actually exercisable) shows a materially faster second build against an unchanged Dockerfile — the first real-world confirmation that `005`'s caching feature delivers its intended benefit, not just that it's reachable.

## Assumptions

- **Open Question** (FR-002, resolve in plan.md): is a JS/Node-based step strictly required to read `$ACTIONS_RUNTIME_TOKEN`/`$ACTIONS_CACHE_URL` from the runner's internal environment, or does a non-JS mechanism exist that this spec's author hasn't identified? This spec's research/plan phase should confirm definitively (reading the actual GitHub Actions runner source, or an authoritative existing explanation) rather than repeat secondhand assumption — the summary's claim that this is JS-only is based on established community precedent (`crazy-max/ghaction-github-runtime`'s own stated purpose, and Dagger's own published GitHub Actions cache-volume guidance recommending exactly that action), not verified firsthand against runner source in this session.
- This spec assumes `005`'s `GitHubActionsCache` implementation itself (the restore-before/save-after logic around the shared Dagger engine's persistent state) is correct and doesn't need changes — only its *inputs* are broken. If real-runner validation (FR-004) surfaces a problem in that logic too, that's a separate follow-up, not silently folded into this spec's scope.
- Consistent with `003`/`005`'s precedent, this spec does not claim `gh-reusable` parity for any part of this fix — `gh-reusable` never had a Dagger-cache-on-GitHub-Actions capability to begin with; this is entirely `paws`-native scope, closing a gap `paws` introduced itself in `005`.

## Validation Plan

- A real GitHub Actions workflow run (in `paws`'s own repo, or a throwaway fixture repo) using the fixed `paws-up`, confirming `CacheBackend::log_line()` prints `"cache: using github-actions"` with no extra consumer-side step — the core proof this spec exists to deliver.
- The same job run a second time against an unchanged Dockerfile, timing-compared against the first run, to confirm SC-003 (a real speed benefit, not just successful detection).
- A test (unit or integration, per FR-007) proving the fix does not cause false-positive `GitHubActionsCache` detection in a non-Actions or Actions-without-legacy-cache-API context.
- If a third-party action is the resolved mechanism (FR-002), confirm the pinned SHA corresponds to a version whose behavior has been manually reviewed, not just "whatever `latest` resolved to at spec-writing time."
- `cargo test --workspace` continues to pass with zero regressions in any existing `paws-dagger` test, and no existing `CacheBackend::detect()` test's expected behavior changes as a side effect of this fix (the function itself is unchanged — only its callers' environment setup changes).

## Rollout and Rollback

- Ships as a `paws-up` (and possibly `paws init`) change — every consumer already depending on `paws-up` picks up the fix automatically on their next run against `version: latest`, or on their next deliberate version bump if pinned (matching how `mbround18/valheim-docker`'s own `PAWS_VERSION` pin works, per the prior migration conversation).
- Zero behavior change for any consumer outside a real GitHub Actions context, or inside one without legacy Cache Service v1 access — `CacheBackend::None` remains correct and unaffected there (FR-005).
- If real-runner validation (FR-004) surfaces a problem this spec didn't anticipate (e.g. the chosen mechanism doesn't actually work the way its precedent suggested), the rollback is reverting `paws-up`'s new step — `GitHubActionsCache` returns to its current `005`-shipped, structurally-unreachable-but-safe state, not a regression below where `paws` is today.
