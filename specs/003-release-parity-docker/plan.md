# Implementation Plan: Full Docker Tag Matrix and In-Repo Changelog

## Inputs

- Spec path: `specs/003-release-parity-docker/spec.md`
- Affected contracts/files:
  - `crates/paws-docker/src/lib.rs` (`generate_tags` internal restructuring, `Cargo.toml` gains `semver`)
  - `crates/paws-changelog/` (new crate: `HistoryProvider` trait + `GitHubHistoryProvider`, entry rendering, file append, commit-back)
  - `crates/paws-core/src/lib.rs` (`PipelineDefaults` gains `changelog_path`)
  - `crates/paws-cli-core/src/lib.rs` (`DockerArgs` new flags; new `Commands::Changelog`/`ChangelogArgs`/`run_changelog`)
  - `Cargo.toml` (workspace `members` gains `crates/paws-changelog`)
  - `README.md` (subcommand list, per Development Workflow's "keep README in sync" rule)
  - Phase 0/1 artifacts: `research.md`, `data-model.md`, `contracts/paws-docker-tag-matrix-contract.md`, `contracts/paws-changelog-contract.md`, `quickstart.md` (all in this directory)

## Constitution Check

_GATE: evaluated before Phase 0 research; re-evaluated below after Phase 1 design._

| Principle | Assessment |
|---|---|
| I. One Crate Per Domain | New domain (`changelog`) gets its own crate (`paws-changelog`), not folded into `paws-docker` or `paws-cli-core`. `paws-cli-core` stays a thin wiring layer — `ChangelogArgs`/`run_changelog` call into `paws-changelog`, no changelog logic lives in `paws-cli-core` itself, matching how `run_semver`/`run_docker` already wire their domain crates. **PASS**. |
| II. Subprocess-First Dagger Access | Neither feature touches Dagger — tag computation is pure string logic, changelog generation is GitHub-REST-API + local file I/O. No new `Command::new("dagger")` call site. **PASS (not applicable)**. |
| III. Incremental SDK Adoption | Not applicable — no Dagger SDK involvement in this feature. **PASS (not applicable)**. |
| IV. Parity Testing Over Reimplementation-From-Memory | Explicitly *not* a parity port (spec Summary) — both third-party actions being replaced are outside `gh-reusable`'s surface. Constitution IV's obligation ("name the exact source function") is satisfied by naming `ghaction-docker-meta`'s `tags:` config and `mbround18/auto`'s observed commit behavior as the *functional* target, while spec's Out of Scope explicitly disclaims format-level parity. **PASS** (spec already reconciles this explicitly). |
| V. Reliability & Testability First | Every new code path (rollup, matrix flags, `HistoryProvider`, commit-back, provider auto-selection) has a named unit test in spec's Validation Plan and this plan's Workstream 3 — no subcommand ships with `(unimplemented)`. **PASS**, contingent on tasks.md actually enumerating each test (tracked in Workstream 3). |
| Tech constraint: no secrets on CLI | `ChangelogArgs` (contracts/paws-changelog-contract.md §1) has no token flag — token resolved via `paws_environment::resolve_github_token`/`CiContext::detect()` only. **PASS**. |
| Tech constraint: shared defaults live in one place | `changelog_path` default lands in `paws-core::PipelineDefaults` (data-model.md), not hardcoded in `paws-changelog`. **PASS**. |
| Tech constraint: no swallowed concurrent failures | Neither feature orchestrates concurrent work (`JoinSet` etc.) — not applicable. **PASS (not applicable)**. |

**Pre-Phase 0 Gate Status**: PASS, no unresolved conflicts.
**Post-Phase 1 Gate Status**: PASS — see re-check at the bottom of this document; Phase 1 design (data-model.md, contracts/) did not introduce anything the table above doesn't already cover.

## Design Decisions

1. **`generate_tags` restructuring is internal-only, not a public signature break** (research.md,
   data-model.md `TagKind`). Build an internal `Vec<TagKind>` representing every applicable tag
   *before* running today's single mirroring loop over it once. `generate_tags`'s existing public
   signature and default (all-new-flags-omitted) output stay byte-identical — SC-001 is a
   regression test running the *existing* `paws-docker` test suite unmodified against the
   restructured code, not a new suite testing the old behavior separately. Alternative considered
   and rejected: keep `generate_tags` untouched and add a second, parallel
   `generate_matrix_tags` function — rejected because it would duplicate the registry-mirroring
   loop (the exact drift risk flagged in spec's Risks section), not just the tag-selection logic.

2. **`paws changelog`'s `HistoryProvider` selection reuses `paws_environment::CiContext::detect()`**
   (research.md R1) rather than inventing new environment-detection logic. This directly
   satisfies the user's stated goal for FR-018 ("provider auto-selection based on environment")
   with an already-shipped, already-tested mechanism, and means a future GitLab provider is added
   by extending `paws_environment::Provider`/`CiContext::detect()` in one place — the same place a
   future GitLab addition to `paws semver`/`paws docker` would also land — rather than maintaining
   parallel detection logic per feature.

3. **Commit/PR-range enumeration uses GitHub's REST compare endpoint** (research.md R2), and
   PR-title lookup reuses `paws-semver::fetch_pr_labels_for_commit`'s exact HTTP-call shape
   against the same `commits/{sha}/pulls` endpoint, reading `.title` instead of `.labels`
   (research.md R3) — a sibling function in `paws-changelog`, not a modification to
   `paws-semver`'s existing function (different crate, different field read, same endpoint).

4. **Commit-back reuses `paws_release::GitHubReleaseClient::get_content`/`put_content` verbatim**
   (research.md R4) — the same mechanism `paws llms generate --publish` already ships
   (`run_llms_generate`, `crates/paws-cli-core/src/lib.rs:2025`). This is not just "a similar
   pattern" — `put_content`'s existing behavior on a stale/conflicting `sha` (a loud
   `anyhow::bail!` with status+body, no retry) **already is** FR-013's Clarifications-resolved
   "fail loudly, no automatic retry" contract, with zero new code required for that specific
   behavior. The `[skip ci]` loop-avoidance marker follows `run_llms_generate`'s own established
   commit-message convention.

5. **PR-ref tags parse the PR number from the existing `git_ref` field** (research.md R5) —
   GitHub Actions' `refs/pull/{number}/merge` — rather than adding a new required CLI input,
   keeping FR-014 purely opt-in-flag-shaped with no new required plumbing through
   `DockerFactsInput`/`GithubContext`.

6. **`paws-docker` gains a `semver` dependency for FR-016's parse** (research.md R6) — pinned to
   the workspace's existing `semver = "1"` (already used by `paws-semver`), not a new external
   dependency to the workspace as a whole. `is_prerelease_version`'s existing substring gate is
   left untouched; the new parse is additive, used only for major/minor extraction on
   already-gated release versions.

7. **Two flags remain implementation decisions deferred to tasks.md, not blocking this plan**:
   (a) exact `--tag-rollup`/`--tag-sha`/`--tag-branch`/`--tag-pr`/`--tag-schedule` flag names are
   locked in contracts/paws-docker-tag-matrix-contract.md §1 and treated as final; (b) the
   `Schedule` tag kind's literal string value (data-model.md's "Open item") is recommended as
   `"schedule"` but is the one remaining format decision tasks.md should confirm before
   implementation, since the spec itself marked it "TBD in plan.md" and this plan is answering it
   with a recommendation rather than a hard requirement.

## Workstreams

1. **`paws-docker` tag matrix** — restructure `generate_tags` internals (`TagKind`, data-model.md);
   add `semver` dependency; implement rollup extraction (FR-001/002/003/004/016), unconditional
   sha tag (FR-015), branch/PR/schedule tags (FR-014) with PR-number/branch-name parsing from
   `git_ref` (R5) and schedule detection from `event_name`; wire new `DockerArgs` flags in
   `paws-cli-core` (contracts/paws-docker-tag-matrix-contract.md §1).
2. **New `paws-changelog` crate** — `HistoryProvider` trait + `GitHubHistoryProvider`
   (`commits_in_range` via compare endpoint, `pr_title_for_commit` via `commits/{sha}/pulls`);
   `ChangelogEntry`/`ChangelogLine` rendering (pure function, no I/O); append-only file writer
   (first-run creation, byte-preservation of prior content); commit-back via
   `paws_release::GitHubReleaseClient`; provider selection via `paws_environment::CiContext`.
   Add crate to workspace `members`.
3. **CLI wiring** — `paws-cli-core::Commands::Changelog(ChangelogArgs)` + `run_changelog`
   (contracts/paws-changelog-contract.md §1); `paws-core::PipelineDefaults::changelog_path`
   (`#[serde(default)]`, data-model.md).
4. **Tests** (Constitution Principle V — pairs with every workstream above, not a separate phase):
   `paws-docker` unit tests per spec's Validation Plan (rollup gating, no-rollup on prerelease/
   non-semver, dedup with `--with-latest`, target-prefix interaction, multi-registry mirroring,
   branch/PR/schedule tag generation, unconditional sha, full-matrix combination, and a
   fixed-snapshot regression test for SC-001's byte-identical-default guarantee); `paws-changelog`
   unit tests (append-only, first-run creation, PR-title rendering, raw-commit-subject fallback,
   prefix-scoped range resolution, commit-back marker/branch/no-op-when-unset/rejected-push,
   `HistoryProvider` auto-selection incl. explicit-error-on-no-match).
5. **Docs/examples** — `README.md` subcommand list gains `paws changelog`; `paws docker --help`
   documents the new flags' scope (spec's Risks: "consumer may assume full parity" mitigation);
   this feature's own `tasks.md` checklist kept in sync as work lands (Development Workflow rule).

## Contract-Safety Checklist

- [x] Workflow declarations and references stay consistent — N/A, no `.github/workflows/*` changes in `paws` itself (downstream `valheim-docker` migration is explicitly out of scope, spec Out of Scope)
- [x] Dagger call names align with module `@func()` names — N/A, no Dagger involvement (Constitution Check table)
- [x] Runtime standards come from `defaults.json`-equivalent — `changelog_path` lands in `paws-core::PipelineDefaults`, not hardcoded (Design Decision 6/data-model.md)
- [x] Permissions are explicit and least-privilege — commit-back's `contents: write` need is named explicitly (spec's Security and Permissions Impact); default (no `--commit`) needs no new scope
- [x] Security implications are documented — spec's Security and Permissions Impact section covers both features; this plan adds no new surface beyond what's already documented there

## Validation Matrix

| Surface | Validation |
| -------------------------- | ---------- |
| `paws-docker` tag computation | `cargo test -p paws-docker` (Workstream 4); regression test asserts SC-001's byte-identical default output against a fixed pre-feature snapshot |
| `paws-changelog` crate | `cargo test -p paws-changelog` (Workstream 4), including a mocked `HistoryProvider` for rendering tests independent of live GitHub calls |
| CLI wiring (`paws-cli-core`) | Existing `paws-cli-core` test conventions (flag parsing / `run_*` smoke tests), extended to `run_changelog` and `DockerArgs`'s new fields |
| End-to-end against `valheim-docker` fixtures | `quickstart.md` §5 — SC-004/SC-005/SC-006, run manually/in a dry-run mode, not part of `cargo test --workspace` (needs the sibling repo's real tag history/`CHANGELOG.md`) |
| Workspace-wide regression | `cargo test --workspace` — zero failures, zero changed expectations in existing `paws-docker`/`paws-semver` tests (spec's Validation Plan, Constitution Principle V) |
