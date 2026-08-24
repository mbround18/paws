# Tasks: Full Docker Tag Matrix and In-Repo Changelog

**Input**: Design documents from `/home/mbruno/development/paws/specs/003-release-parity-docker/`
(`plan.md`, `spec.md`, `research.md`, `data-model.md`, `contracts/`, `quickstart.md`)

## Rules

- Keep changes backward-compatible unless explicitly declared breaking (FR-005: default `paws docker` output byte-identical when no new flag is passed).
- Pair contract changes with tests in the same PR (Constitution Principle V).
- Keep docs/examples (`README.md`, `--help` text) in sync with behavior (Development Workflow).
- `cargo test --workspace` MUST pass with zero failures before this feature is considered done.

## Phase 1: Setup

- [X] T001 Add `semver.workspace = true` to `crates/paws-docker/Cargo.toml` (research.md R6)
- [X] T002 Create `crates/paws-changelog/` crate skeleton (`Cargo.toml` with `anyhow`, `tokio`, `serde`, `serde_json`, `reqwest`, `schemars`, `async-trait`, `paws-environment`, `paws-release`, `paws-semver` dependencies matching sibling-crate conventions; empty `src/lib.rs`) and add `"crates/paws-changelog"` to `Cargo.toml`'s workspace `members`

## Phase 2: Foundational (blocking prerequisites)

**Purpose**: shared plumbing both P1 Docker stories (US1, US3) need before any tag-kind-specific work, plus the shared-defaults field the changelog crate needs — must complete before Phase 3+.

- [X] T003 Add `changelog_path: Option<String>` field (with `#[serde(default)]`, per data-model.md) to `PipelineDefaults` in `crates/paws-core/src/lib.rs`, and extend its existing `defaults_roundtrip_through_json` test to cover the new field
- [X] T004 Restructure `paws-docker::generate_tags` internals in `crates/paws-docker/src/lib.rs` to build an internal `Vec<TagKind>` (data-model.md) before running the existing per-tag/per-registry mirroring loop once over it — public signature and default (no new flags) output MUST stay byte-identical (FR-005)
- [X] T005 [P] Add a fixed-snapshot regression test in `crates/paws-docker/src/lib.rs` capturing `generate_tags`'s pre-feature output for a representative fixture set, asserting it is unchanged after T004's restructuring (SC-001)
- [X] T006 [P] Add PR-number parsing (`refs/pull/{number}/merge` → `u64`) and branch-name parsing (`refs/heads/{branch}` → `String`) helper functions alongside the existing `is_git_sha`/`is_prerelease_version` helpers in `crates/paws-docker/src/lib.rs` (research.md R5)

**Checkpoint**: `cargo test -p paws-docker` and `cargo test -p paws-core` pass; `generate_tags`'s default output is provably unchanged before any story-specific tag kind is added.

---

## Phase 3: User Story 1 - Publish major/major.minor rollup tags on a Docker release (Priority: P1)

**Goal**: `paws docker --tag-rollup` emits `major`/`major.minor` tags alongside the existing version tag for release-quality versions, mirrored across registries, gated identically to today's `latest` gate.

**Independent Test**: run `paws docker --version v3.2.1 --tag-rollup` against a tag-ref fixture; assert `image:v3.2.1`, `image:3.2`, `image:3` (plus registry mirrors) appear, and that `--version v3.2.1-rc.1` on the same fixture produces no rollup tags.

- [X] T007 [P] [US1] Unit test: `--tag-rollup` on a release version (`v3.2.1`) produces `3` and `3.2` rollup tags in `crates/paws-docker/src/lib.rs`
- [X] T008 [P] [US1] Unit test: `--tag-rollup` omitted produces byte-identical output to `generate_tags`'s pre-feature baseline (FR-005) in `crates/paws-docker/src/lib.rs`
- [X] T009 [P] [US1] Unit test: `--tag-rollup` on a prerelease version (`v3.2.1-rc.1`) produces zero rollup tags (FR-002) in `crates/paws-docker/src/lib.rs`
- [X] T010 [P] [US1] Unit test: `--tag-rollup` on a version that fails semver parsing (build metadata `v3.2.1+abc`, bare git sha) produces zero rollup tags, not a malformed one (FR-016) in `crates/paws-docker/src/lib.rs`
- [X] T011 [P] [US1] Unit test: `--tag-rollup` + `--with-latest` together produce no duplicate tags (FR-006) in `crates/paws-docker/src/lib.rs`
- [X] T012 [P] [US1] Unit test: `--tag-rollup` + `--target`/`--prepend-target` produces `odin-3`/`odin-3.2`, not bare `3`/`3.2` (FR-004) in `crates/paws-docker/src/lib.rs`
- [X] T013 [P] [US1] Unit test: `--tag-rollup` tags are mirrored across every `--registries` entry via the existing mirroring loop (FR-003) in `crates/paws-docker/src/lib.rs`
- [X] T014 [US1] Implement `TagKind::RollupMajor`/`RollupMinor` construction: gated on `is_release_version` and a successful `semver::Version::parse`, using T006's helpers where applicable, in `crates/paws-docker/src/lib.rs` (FR-001, FR-002, FR-016) — makes T007–T013 pass
- [X] T015 [US1] Add `--tag-rollup` flag to `DockerArgs` in `crates/paws-cli-core/src/lib.rs` and thread it through `resolve_docker_facts`/`run_docker` into T014's construction

**Checkpoint**: User Story 1 independently functional and testable — `paws docker --tag-rollup` works end-to-end.

---

## Phase 4: User Story 3 - Full Docker tag matrix to retire `ghaction-docker-meta` outright (Priority: P1)

**Goal**: `paws docker` can additionally emit branch-ref, PR-ref, schedule, and unconditional sha tags, each opt-in and each flowing through the same mirroring path as every other tag — the full matrix `docker-release.yml`/`docker-build.yml` need to drop `ghaction-docker-meta` entirely.

**Independent Test**: run `paws docker` with each new flag enabled against a fixture for its matching trigger shape (branch push, PR, schedule, tag) and assert the corresponding tag type appears; run rollup+sha+latest together and assert no duplicates/cross-interference.

- [X] T016 [P] [US3] Unit test: `--tag-branch` on a branch-push build (`git_ref = refs/heads/some-branch`) produces a branch-derived tag, mirrored across registries (FR-014) in `crates/paws-docker/src/lib.rs`
- [X] T017 [P] [US3] Unit test: `--tag-pr` on a `pull_request` build produces a `pr-{number}`-shaped tag (FR-014, using T006's PR-number parse) in `crates/paws-docker/src/lib.rs`
- [X] T018 [P] [US3] Unit test: `--tag-schedule` on a scheduled-trigger build (`event_name == "schedule"`) produces the `schedule` tag (FR-014; confirm literal string per plan.md Design Decision 7 — recommended `"schedule"`) in `crates/paws-docker/src/lib.rs`
- [X] T019 [P] [US3] Unit test: `--tag-sha` produces a `sha`-prefixed tag unconditionally alongside other tag types, not only as the fallback primary tag (FR-015) in `crates/paws-docker/src/lib.rs`
- [X] T020 [US3] Unit test: `--tag-rollup` + `--tag-sha` + `--with-latest` together produce the full set with no duplicates and no cross-type interference (User Story 3 Acceptance Scenario 5) in `crates/paws-docker/src/lib.rs`
- [X] T021 [US3] Implement `TagKind::BranchRef`/`PrRef`/`Schedule` construction, each gated on the matching `event_name`/`git_ref` trigger shape (not just the flag alone), and unconditional `TagKind::Sha` construction, in `crates/paws-docker/src/lib.rs` (FR-014, FR-015) — makes T016–T020 pass
- [X] T022 [US3] Add `--tag-sha`, `--tag-branch`, `--tag-pr`, `--tag-schedule` flags to `DockerArgs` in `crates/paws-cli-core/src/lib.rs` and thread through to T021's construction
- [X] T023 [US3] Document the new flags' exact scope (which tag types, which trigger shapes) in `paws docker --help` text in `crates/paws-cli-core/src/lib.rs` (spec Risks: mitigates "consumer assumes full parity")

**Checkpoint**: User Stories 1 and 3 together let `docker-release.yml`/`docker-build.yml` drop `crazy-max/ghaction-docker-meta` entirely (SC-005).

---

## Phase 5: User Story 2 - Maintain a `CHANGELOG.md` across releases (Priority: P2)

**Goal**: a standalone `paws changelog` subcommand generates a changelog entry from commit/PR history between two refs, appends it to `CHANGELOG.md`, and optionally commits it back to the default branch with a loop-avoidance marker — enough to drop `mbround18/auto` entirely.

**Independent Test**: run against a fixture repo with two tags and merged-PR commits between them; assert a new dated section is appended listing each commit/PR, and pre-existing file content is untouched byte-for-byte.

- [X] T024 [P] [US2] Define `HistoryProvider` trait and `HistoryCommit` struct in `crates/paws-changelog/src/lib.rs` (data-model.md, contracts/paws-changelog-contract.md §3)
- [X] T025 [P] [US2] Define `ChangelogEntry`/`ChangelogLine` structs and a pure (no I/O) Markdown-section rendering function in `crates/paws-changelog/src/lib.rs` (FR-008) — `date` field is a manually formatted `String` (`YYYY-MM-DD`, via `std::time`), not a new `chrono` dependency (resolved per analysis finding U1)
- [X] T026 [US2] Implement `GitHubHistoryProvider::commits_in_range` via GitHub's compare-two-commits REST endpoint in `crates/paws-changelog/src/lib.rs` (research.md R2)
- [X] T027 [US2] Implement `GitHubHistoryProvider::pr_title_for_commit` via `GET /repos/{owner}/{repo}/commits/{sha}/pulls`, mirroring `paws-semver::fetch_pr_labels_for_commit`'s call shape but reading `.title` (research.md R3) in `crates/paws-changelog/src/lib.rs`
- [X] T028 [US2] Implement `HistoryProvider` auto-selection via `paws_environment::CiContext::detect()`, with an explicit actionable error when no provider's environment signature matches (FR-018) in `crates/paws-changelog/src/lib.rs`
- [X] T029 [US2] Implement commit-range resolution reusing `paws_semver::resolve_last_tag` for the prefix-aware "previous" tag (FR-010) in `crates/paws-changelog/src/lib.rs`
- [X] T030 [US2] Implement the append-only `CHANGELOG.md` writer: read-or-treat-as-empty, append T025's rendered section, write back, preserving all pre-existing bytes above the new section (FR-007, FR-008) in `crates/paws-changelog/src/lib.rs`
- [X] T031 [US2] Implement commit-back via `paws_release::GitHubReleaseClient::get_content`/`put_content`, with a commit message carrying the `[skip ci]` loop-avoidance marker unconditionally, relying on `put_content`'s existing loud-fail-on-conflict behavior for the no-retry requirement (FR-013) in `crates/paws-changelog/src/lib.rs`
- [X] T032 [US2] Add `Commands::Changelog(ChangelogArgs)` and `run_changelog` in `crates/paws-cli-core/src/lib.rs`, wiring `--version`, `--previous-ref`, `--prefix`, `--output` (falling back to `PipelineDefaults::changelog_path`), `--commit`, `--repository`, `--branch` per contracts/paws-changelog-contract.md §1 — T024–T032 collectively make T033–T041 pass
- [X] T033 [P] [US2] Unit test: append-only behavior against a pre-populated fixture `CHANGELOG.md` — copy `valheim-docker`'s real `CHANGELOG.md` into `crates/paws-changelog/tests/fixtures/` per SC-003 — in `crates/paws-changelog/src/lib.rs`
- [X] T034 [P] [US2] Unit test: first-run file creation when no `CHANGELOG.md` exists yet (not an error) in `crates/paws-changelog/src/lib.rs`
- [X] T035 [P] [US2] Unit test: PR-title rendering against a mocked `HistoryProvider` in `crates/paws-changelog/src/lib.rs`
- [X] T036 [P] [US2] Unit test: raw-commit-subject fallback when no PR is found for a commit (FR-009) in `crates/paws-changelog/src/lib.rs`
- [X] T037 [P] [US2] Unit test: prefix-scoped range resolution in a simulated monorepo fixture (e.g. `chart-name-v1.2.0`-style tags) (FR-010) in `crates/paws-changelog/src/lib.rs`
- [X] T038 [P] [US2] Unit test: commit-back message contains the loop-avoidance marker and the commit targets the given default branch (FR-013) in `crates/paws-changelog/src/lib.rs`
- [X] T039 [P] [US2] Unit test: commit-back is a no-op when `--commit` is unset, matching FR-011's default-no-write behavior in `crates/paws-changelog/src/lib.rs`
- [X] T040 [P] [US2] Unit test: a rejected commit-back push fails loudly with no automatic retry, leaving the entry text already printed on stdout (Clarifications Session 2026-08-23) in `crates/paws-changelog/src/lib.rs`
- [X] T041 [P] [US2] Unit test: `HistoryProvider` auto-selection picks the GitHub provider on a GitHub environment signature, and produces the documented explicit error (not a silent no-op) when no signature matches (FR-018) in `crates/paws-changelog/src/lib.rs`

**Checkpoint**: User Story 2 independently functional — `paws changelog` (with or without `--commit`) works end-to-end against a real or fixture repo.

---

## Phase 6: Polish & Cross-Cutting Concerns

- [X] T042 [P] Update `README.md`'s subcommand list to include `paws changelog` (Development Workflow: "keep README's subcommand list ... in sync")
- [X] T043 Run `cargo test --workspace` and confirm zero failures and zero changed expectations in any pre-existing `paws-docker`/`paws-semver` test (spec Validation Plan; Constitution Principle V)
- [X] T044 Run the end-to-end dry-run scenarios in `quickstart.md` §5 against `valheim-docker`'s real tag history and `CHANGELOG.md` fixture, confirming SC-004, SC-005, and SC-006 concretely — SC-004 (`image:3` round-trip for `mbround18/valheim v3.6.1`) and SC-005 (tag-type superset across `docker-release.yml`'s branch/PR/schedule/tag trigger shapes for both `odin`/`valheim`) verified offline via `generate_tag_matrix` directly; SC-006's local-write half verified against the real 10374-byte `valheim-docker` `CHANGELOG.md` (all pre-existing bytes preserved, new section appended correctly). SC-006's commit-back half needs a live GitHub token against a scratch repo (quickstart.md §4's own explicit precondition) — not run in this session; remains a manual step for whoever has those credentials, per quickstart.md's guidance not to run `--commit` against a real repo without one

## Dependencies & Execution Order

- **Phase 1 (Setup)** → **Phase 2 (Foundational)**: strictly sequential; T004 (the `generate_tags` restructuring) blocks every Docker task in Phase 3/4; T003 (`PipelineDefaults`) blocks Phase 5's `--output` fallback (T032).
- **Phase 3 (US1)** and **Phase 4 (US3)** both depend only on Phase 2, not on each other — they touch the same file (`crates/paws-docker/src/lib.rs`) but different `TagKind` variants, so they can be implemented in either order once Phase 2 is done. Both are P1 and both are required together to satisfy SC-005 ("full retirement" bar), but each is independently testable per its own Independent Test.
- **Phase 5 (US2)** depends only on Phase 2 (specifically T003) — it does not depend on Phase 3 or Phase 4 at all (separate crate, separate CLI subcommand). It can be built in parallel with Phase 3/4 by a different contributor/session.
- **Phase 6 (Polish)** depends on all of Phase 3, 4, and 5 being complete.

```
Setup (T001-T002)
    ↓
Foundational (T003-T006)
    ↓
    ├── US1 (T007-T015) ──┐
    ├── US3 (T016-T023) ──┼── Polish (T042-T044)
    └── US2 (T024-T041) ──┘
```

## Parallel Execution Examples

- **Within Phase 2**: T005 and T006 can run in parallel (different concerns, same file — coordinate on merge order, not blocking logic) once T004 lands.
- **Within Phase 3 (US1)**: T007–T013 (all test tasks) can be written in parallel by different agents/sessions before T014/T015 land, since they exercise `generate_tags` from the outside and don't depend on each other's code.
- **Within Phase 4 (US3)**: T016–T019 likewise parallelizable ahead of T021/T022.
- **Within Phase 5 (US2)**: T024 and T025 (pure data/rendering, no network) can start immediately after Phase 2; T033–T041 (tests) can be drafted in parallel once T024/T025's shapes are fixed, even before T026–T032 are fully implemented, using a mocked `HistoryProvider`.
- **Across phases**: Phase 3+4 (Docker) and Phase 5 (changelog) touch disjoint crates (`paws-docker`/`paws-cli-core`'s `DockerArgs` vs. `paws-changelog`/`paws-cli-core`'s `ChangelogArgs`) and can be worked by two different contributors/sessions simultaneously once Phase 2 is merged.

## Implementation Strategy

**MVP scope**: User Story 1 alone (T001–T015, skipping T002/T003's changelog-only setup) is independently shippable and immediately unblocks the sharpest pain point — consumers pinning to `image:3` — without needing the full tag matrix or the changelog subcommand. Recommended as the first PR.

**Incremental delivery after MVP**:
1. Ship User Story 1 (rollup tags) — unblocks major-tag-pinning consumers.
2. Ship User Story 3 (full tag matrix) — together with US1, this is what actually lets `docker-release.yml`/`docker-build.yml` delete `crazy-max/ghaction-docker-meta` (the spec's stated bar, SC-005). Both P1 stories should land before that deletion happens downstream, even though each merges independently.
3. Ship User Story 2 (changelog) — lower blast radius (P2); can land before or after US1/US3 since it's a fully separate crate and CLI subcommand with no shared code path.
4. Polish (README, end-to-end validation against `valheim-docker`) once all three stories are in.
