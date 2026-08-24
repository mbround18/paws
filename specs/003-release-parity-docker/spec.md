# Feature Specification: Full Docker Tag Matrix and In-Repo Changelog — Retiring `ghaction-docker-meta` and `mbround18/auto`

**Feature Branch**: `003-release-parity-docker`

**Created**: 2026-08-23

**Status**: Draft

**Input**: User description: "While evaluating a migration of `mbround18/valheim-docker`'s GitHub Actions CI to `paws`, two behaviors that repo's current workflows depend on have no `paws` equivalent: (1) a full semver tag rollup (`major`, `major.minor`, full version, plus branch/PR/sha/schedule tags) on Docker image publish, currently produced by `crazy-max/ghaction-docker-meta`; and (2) a maintained `CHANGELOG.md` file updated on every release, currently produced by `mbround18/auto`. Spec what `paws` would need to add to close both gaps." **Revised goal (explicit)**: the target is not partial gap-closing that still leaves both third-party actions in the workflow for the parts `paws` doesn't cover — the target is for `paws` to fully replace `crazy-max/ghaction-docker-meta` and `mbround18/auto` so `valheim-docker`'s workflows can drop both actions entirely.

## Clarifications

### Session 2026-08-23

- Q: Should changelog generation be a standalone `paws changelog` subcommand, or a flag on `paws semver --push`? → A: Standalone `paws changelog` subcommand (usable on its own or chained after `paws semver --push`).
- Q: How should the changelog subcommand enumerate commits/PRs between two refs — local git, or a remote API? → A: A remote API (GitHub only, for this spec), but behind a `HistoryProvider` abstraction with auto-selection by environment, so GitLab and a local-git provider can be added later without changing `paws changelog`'s CLI contract.
- Q: What should `paws changelog --commit` do if pushing the `CHANGELOG.md` commit to the default branch is rejected (e.g. a race with another commit)? → A: Fail loudly (non-zero exit, clear error) with no automatic retry; the generated entry text on stdout is the fallback for the caller to apply manually.

## Summary

Two behaviors `mbround18/valheim-docker` depends on today have no `paws` equivalent and block a full migration off both third-party actions:

1. **Docker tag matrix**: `paws docker`'s `generate_tags` (ported 1:1 from `gh-reusable`'s `docker-facts::generateTags`) is a *select-one* function — it computes exactly one primary tag (version or sha) plus an optional `latest`, mirrored per registry. `crazy-max/ghaction-docker-meta`, as configured in `valheim-docker`'s `docker-release.yml`, is an *emit-all-applicable* function — on a single build it can simultaneously produce a `schedule` tag, a branch-ref tag, a PR-ref tag, all three semver variants (`{version}`, `{major}.{minor}`, `{major}`), and a `sha` tag. Fully retiring `ghaction-docker-meta` requires `paws docker` to adopt the same emit-all-applicable model, not just add two more single-purpose flags. Consumers who pin to `image:3` (major-only) for stability — a pattern `valheim-docker`'s own README documents and recommends — are the sharpest edge of this gap, but the full matrix (branch/PR/schedule tags used by `docker-build.yml`'s canary-image flow) also needs a home in `paws` for the action to go away entirely.
2. **Changelog generation**: `paws semver --push` creates a git tag and a GitHub Release with `generate_release_notes: true` (GitHub's auto-generated, PR-title-derived notes attached to the Release page). It does not write, update, or commit a `CHANGELOG.md` file back to the repo. `valheim-docker`'s current release flow (`mbround18/auto` via `release.yml`) does exactly that: it appends a new dated section to `CHANGELOG.md` and commits it straight to `main` with the message `Update CHANGELOG.md [skip ci]` — and `release.yml`'s own trigger guard (`!contains(github.event.head_commit.message, 'ci skip') && !contains(..., 'skip ci')`) exists specifically to stop that commit from re-triggering the release workflow. Fully retiring `mbround18/auto` means `paws` must reproduce both halves of this: the write, and the loop-avoidance convention that makes the write safe on a `push`-triggered workflow.

This spec defines new `paws` behavior to fully close both gaps so both third-party actions can be dropped. **Neither gap is a `gh-reusable` parity bug** — `gh-reusable`'s own `docker-facts::generateTags` has the identical single-tag-plus-latest scope (confirmed by reading `actions/docker-facts/src/lib.ts`'s `generateTags`), and `gh-reusable` has no changelog-writing capability anywhere in its `@func()` surface. Both gaps come from third-party GitHub Actions (`crazy-max/ghaction-docker-meta`, `mbround18/auto`) that `valheim-docker` layered on top of its previous CI, outside anything `paws` has ever claimed to port. Per Constitution Principle IV ("Parity Testing Over Reimplementation-From-Memory"), this spec does **not** claim parity with either third-party action's exact output format — it defines new `paws`-native behavior that covers the same *functional* ground (every tag type, a committed changelog with loop-safe automation) well enough for full removal, and calls out every point where the new behavior intentionally differs in format or mechanism from what those actions currently produce.

## Motivation and Problem Statement

- **Silent regression risk**: if a consumer repo migrates its Docker publish workflow to `paws docker` without this feature, its major/major.minor tags simply stop being published on the next release with no error — existing `image:3` references become stale instead of failing loudly. This is the kind of silent contract change Constitution Principle V's testability bar exists to prevent.
- **Changelog loss is a regression, not a simplification**: `CHANGELOG.md` is a durable, versioned, greppable record; GitHub's `generate_release_notes` output lives only on the Releases page and is not part of the repo's history. A consumer that drops `mbround18/auto` for `paws semver --push` loses that artifact entirely unless `paws` grows an equivalent.
- **Both gaps block the same real migration**: they were both found while spec'ing out a `paws` adoption plan for `mbround18/valheim-docker`'s four workflows (`docker-build.yml`, `docker-release.yml`, `release.yml`, `enforce-labels.yml`). `docker-build.yml`/`docker-release.yml`/`release.yml` cannot move to `paws` without a plan for these two; this spec is that plan.
- **The bar is deletion, not coexistence**: a partial migration that leaves `crazy-max/ghaction-docker-meta` or `mbround18/auto` in the workflow to cover whatever `paws` doesn't isn't the target outcome here — the target is a `docker-release.yml`/`release.yml` with both third-party actions' steps removed entirely. That's why this spec's scope covers the full tag matrix (User Story 3) and the changelog commit-back mechanism (FR-013), not just the narrowest slice that avoids the *silent* regression.

## Scope

### In scope

- A new opt-in tag-matrix mode for `paws docker` that, for a release-quality version (see FR-001's release-version definition, matching `generate_tags`'s existing `is_release_version` check), additionally emits `major` and `major.minor` tags alongside the existing full-version tag, mirrored across every configured registry the same way existing tags are.
- Extending `paws docker`'s tag computation from a *select-one-primary-tag* model to an *emit-all-applicable* model: branch-ref tags (event=branch builds), PR-ref tags (event=pull_request builds), a `schedule` tag (scheduled builds), and an unconditional `sha` tag alongside whatever else applies — all opt-in, all additive to what a given build already produces today. This is the change needed for `docker-build.yml`'s canary/branch image flow and `docker-release.yml`'s full-matrix flow to drop `crazy-max/ghaction-docker-meta` outright.
- A new standalone `paws changelog` subcommand (see Clarifications, Session 2026-08-23) that generates a changelog entry for a version bump from the commit/PR history between two refs, writes it into a `CHANGELOG.md` in the target repo, and — opt-in, see FR-013 — commits that file back to the repo using a documented loop-avoidance marker in the commit message, so a `push`-triggered release workflow doesn't re-trigger itself. This is the change needed to drop `mbround18/auto` outright rather than just approximate its Markdown formatting.
- A `HistoryProvider` abstraction (FR-017–FR-018, see Clarifications) behind which the commit/PR-range lookup runs, with a GitHub implementation shipped in this spec and the interface shaped so a GitLab or local-`git log` provider can be added later without a `paws changelog` CLI change. Provider selection is automatic based on environment (e.g. detecting GitHub-specific environment/token shape), not a new CLI flag the caller has to set.
- Explicit documentation, in both features, of every point where the new `paws`-native output differs from `ghaction-docker-meta`'s or `mbround18/auto`'s current output — this spec treats "different but good enough to migrate onto" as an acceptable, *declared* outcome, not a bug to hide.

### Out of scope

- Implementing a GitLab or local-`git log` `HistoryProvider` — this spec defines the provider abstraction and auto-selection mechanism (FR-017–FR-018) and ships exactly one implementation (GitHub). Additional providers are follow-up work that should not require changing `paws changelog`'s CLI contract when they land.
- Reproducing `ghaction-docker-meta`'s exact tag *string* formats where they diverge from `paws` conventions already in use elsewhere (e.g. `generate_tags`'s existing `sha-`-prefix convention is kept rather than switching to `ghaction-docker-meta`'s bare-sha format) — functional coverage of every tag *type* is in scope; byte-identical tag strings for types `paws` didn't previously touch are not.
- Reproducing `mbround18/auto`'s exact changelog Markdown formatting, section headers, or its own label-to-category mapping byte-for-byte. This spec defines a new, `paws`-native changelog format (see FR-008) good enough to serve the same purpose, not a clone.
- Changing `paws semver`'s existing version-computation logic (label/branch inference, prerelease handling) — this feature only adds what happens *after* a version is computed.
- Any non-Docker package-registry tag scheme (crates.io, npm, Helm chart versions already have their own versioning conventions and are unaffected).
- Migrating `valheim-docker`'s actual workflow YAML — that is downstream consumer work once this spec ships, tracked separately.

## Affected Contracts

- **`paws docker` CLI contract**: new flags (see FR-001, FR-013–FR-016) added to the existing subcommand; default behavior for existing callers is unchanged (no new tags appear unless a flag is passed) — an additive, non-breaking contract change per Constitution's Development Workflow guidance on pre-1.0 backward compatibility.
- **`paws-docker::generate_tags` contract**: this is the one non-additive internal change in this spec. Today's signature returns a single computed tag set from a select-one-primary-tag model; supporting the full matrix (branch/PR/schedule/sha, all simultaneously applicable) requires either restructuring `generate_tags` internally to build a *set* of applicable tag types before mirroring, or introducing a new higher-level function that composes multiple `generate_tags`-style calls. Either way, **existing callers that pass none of the new opt-in flags MUST see byte-identical output to today** (FR-005) — the restructuring is an internal implementation concern, not a contract break, but it is a bigger internal change than "add a flag" and should be named as such in plan.md's design section.
- **New `paws changelog` subcommand contract** (standalone, per Clarifications Session 2026-08-23): inputs are a version, a previous ref/tag, a target changelog file path, and provider credentials (for GitHub, a token, reusing `paws-semver`'s existing `fetch_pr_labels_for_commit`-style access pattern) resolved through the `HistoryProvider` abstraction (FR-017–FR-018); output is a written/updated `CHANGELOG.md` plus the generated entry text on stdout for callers that want it elsewhere (e.g. a release-notes body). When the commit-back flag (FR-013) is set, the contract also includes a git commit against the target repo's default branch. Being standalone, it can be invoked independently of `paws semver --push` to preview or regenerate an entry.
- **New `HistoryProvider` trait/interface contract** (internal to the changelog capability, FR-017–FR-018): defines whatever operations changelog generation needs from a commit/PR history source (range lookup, PR-title lookup with raw-commit-subject fallback). This spec's only concrete implementer is a GitHub provider; the interface itself, not any particular implementation, is the durable contract future providers must satisfy.
- **No existing `gh-reusable` contracts change** — as established in Summary, neither of these behaviors has a `gh-reusable` source to stay in parity with.

## Runtime and Defaults Impact

- No new required environment variables for the Docker rollup feature — it reuses `paws docker`'s existing version/registry/push inputs.
- The changelog feature needs a GitHub token for commit→PR lookups (same requirement `paws semver --push` already has for creating tags/releases) — no *new* secret category, just an existing one used by one more code path.
- `paws-core::PipelineDefaults` gains no new fields for the Docker rollup (it's pure tag-string derivation, no registry/toolchain defaults involved). The changelog feature's default output path (`CHANGELOG.md` at repo root) should live in `PipelineDefaults` rather than be hardcoded in the new crate/module, per the Technical Constraints in the constitution ("Shared defaults live in one place").

## Security and Permissions Impact

- The Docker tag-matrix feature has no security impact beyond what `paws docker` already has — it only changes which tag *strings* get pushed, not push gating or registry auth. Branch/PR-ref tags do introduce a new *information* consideration worth documenting: a PR-ref tag publishes an image built from PR head content under a predictable tag name (`pr-{number}`), same as `ghaction-docker-meta`'s behavior today — not a new exposure, but worth naming since it's new surface for `paws docker` specifically.
- The changelog feature performs read-only GitHub API calls (listing commits/PRs between two refs) plus a local file write — no new write-scope GitHub permission is needed beyond what `paws semver --push` already requires to create the tag/release, **when the commit-back flag (FR-013) is not set**.
- **When FR-013's commit-back flag *is* set** (required to fully retire `mbround18/auto`, since that's what it does today), the feature needs `contents: write` against the target repo's default branch to push the commit — `paws semver --push`'s tag/release creation already implies this scope on the token, so no *new* permission category is introduced, but the write target changes from "a tag ref" to "the default branch's tip," which is a meaningfully different blast radius (a bad commit here lands directly on `main`, not on a ref a maintainer can just re-tag). FR-013 should require the loop-avoidance marker (`[skip ci]` or equivalent, matching `valheim-docker`'s existing convention) unconditionally whenever commit-back is enabled, not as a separate opt-in, since an unmarked commit-back on a `push`-triggered release workflow is a self-triggering loop, not just a missing nicety.

## Risks and Mitigations

- **Risk**: `generate_tags`'s select-one-primary-tag internal model doesn't cleanly extend to emit-all-applicable without either a breaking signature change or duplicated tag-mirroring logic between the old and new code paths, risking the two paths drifting (e.g. registry-mirroring bug fixed in one but not the other).
  **Mitigation**: FR-014 requires all tag types (existing and new) to flow through the exact same per-tag registry-mirroring loop `generate_tags` already has — no separate mirroring implementation for the new tag types, verified by a shared-path unit test (FR-014).
- **Risk**: Rollup tag generation for a version like `v3.2.1-rc.1` (prerelease) accidentally publishes a `3` or `3.2` tag, clobbering the real major/minor pointer with a prerelease build.
  **Mitigation**: FR-002 requires rollup tags to use the exact same `is_release_version` gate `generate_tags` already applies to `latest` — prereleases never get rollup tags, mirroring existing, already-tested logic rather than inventing a new gate.
- **Risk**: A `paws docker --tag-rollup` version string doesn't cleanly decompose into `MAJOR.MINOR.PATCH` (e.g. build metadata like `v3.2.1+abc`, or a version with fewer/more than three dot-separated components) — naive string-splitting produces a wrong or malformed rollup tag instead of failing loudly.
  **Mitigation**: FR-002 requires major/minor extraction to go through an actual semver parse (this introduces a new `semver` crate dependency to `paws-docker`, which does not parse versions at all today — see plan.md design note) rather than ad hoc splitting; a version that fails to parse produces no rollup tags rather than a malformed one.
- **Risk**: The new changelog format doesn't match what a consumer's existing `CHANGELOG.md` history looks like (formatted by `mbround18/auto` for every prior release), producing a jarring format break mid-file.
  **Mitigation**: FR-008 requires the feature to *append* a new dated section rather than rewrite the file, so prior entries are preserved verbatim; only new entries use the new format. Document this as an expected one-time format transition in the feature's own docs.
- **Risk**: Commit/PR-history-based changelog generation misattributes or omits entries when a repo doesn't use PR-based merges (e.g. direct pushes to main).
  **Mitigation**: FR-009 requires a documented, deterministic fallback (raw commit subject) when no associated PR is found for a commit in range, rather than silently dropping that commit from the changelog.
- **Risk**: Changelog commit-back (FR-013) re-triggers the very `push`-triggered release workflow that ran it, because the commit lacks (or the workflow doesn't check for) a loop-avoidance marker — an infinite release loop, not a cosmetic bug.
  **Mitigation**: FR-013 requires the commit message to unconditionally include a documented marker (matching `valheim-docker`'s existing `[skip ci]` convention) whenever commit-back is enabled; this spec documents that the *consumer's* workflow trigger condition (like `release.yml`'s existing `!contains(..., 'skip ci')` guard) is still the consumer's responsibility to keep in place — `paws` can emit the marker but cannot force a caller's workflow YAML to respect it.
- **Risk**: The default branch moves between `paws changelog --commit` reading its tip and pushing the new commit (e.g. a concurrent release, or a manual push), causing the push to be rejected.
  **Mitigation**: FR-013 requires a loud, non-zero-exit failure with no automatic retry (see Clarifications, Session 2026-08-23) rather than a silent no-op or a retry loop that could mask a real conflict; the entry text already on stdout lets the caller apply it manually or re-run the command.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Publish major/major.minor rollup tags on a Docker release (Priority: P1)

As a maintainer publishing a versioned Docker image, I want `paws docker` to optionally publish `major` and `major.minor` rollup tags (e.g. `myimage:3` and `myimage:3.2` alongside `myimage:v3.2.1`) so consumers who pin to a major version for stability (a pattern this project's own docs already recommend) have something to pin to after migrating off `ghaction-docker-meta`.

**Why this priority**: this is the harder blocker of the two — without it, a consumer with published major-pinned image references cannot migrate to `paws docker` without breaking every downstream user of `image:3`.

**Independent Test**: run `paws docker --version v3.2.1 --tag-rollup` against a fixture with `git_ref` on a real tag ref; assert the resulting tag list contains `image:v3.2.1`, `image:3.2`, and `image:3` (plus registry-mirrored variants of each), and that a second run with `--version v3.2.1-rc.1` (prerelease) on the same fixture produces *no* rollup tags, only the full prerelease tag.

**Acceptance Scenarios**:

1. **Given** `--version v3.2.1`, a tag `git_ref`, and `--tag-rollup` enabled, **When** `paws docker` computes tags, **Then** the output includes `{image}:v3.2.1`, `{image}:3.2`, and `{image}:3`, each also mirrored into every configured `--registries` entry exactly as existing tags are (per `generate_tags`'s current per-tag registry mirroring loop).
2. **Given** the same inputs but `--tag-rollup` omitted, **When** `paws docker` computes tags, **Then** output is byte-identical to today's `generate_tags` output — no rollup tags appear by default.
3. **Given** `--version v3.2.1-rc.1` (a prerelease per the existing `is_prerelease_version` check) and `--tag-rollup` enabled, **When** tags are computed, **Then** no `3` or `3.2` rollup tag is produced, matching `is_release_version`'s existing gate on `latest`.
4. **Given** `--with-latest` and `--tag-rollup` both enabled on a release version, **When** tags are computed, **Then** `latest`, `3`, `3.2`, and `v3.2.1` are all present with no duplicates.
5. **Given** `--target`/`--prepend-target` is set (multi-stage build tagging, e.g. `odin-v3.2.1`), **When** rollup tags are computed, **Then** rollup tags respect the same target-prefix behavior as the full-version tag (`odin-3`, `odin-3.2`), not a bare `3`/`3.2`.

---

### User Story 2 - Maintain a `CHANGELOG.md` across releases (Priority: P2)

As a maintainer running `paws semver --push` to cut a release, I want a `CHANGELOG.md` entry generated and written for that version from the commit/PR history since the last tag, so my repo keeps a durable, in-repo release history without a separate third-party changelog action.

**Why this priority**: real, but lower blast radius than User Story 1 — a missing changelog is a documentation gap a maintainer can work around manually for a release or two; a missing major-pin tag breaks other people's builds immediately.

**Independent Test**: run the changelog feature against a fixture repo with two tags and a handful of merged-PR commits in between; assert a new dated section is appended to `CHANGELOG.md` listing each commit/PR in range, and that pre-existing file content above the new section is untouched byte-for-byte.

**Acceptance Scenarios**:

1. **Given** a repo with a previous tag `v1.2.0`, new tag `v1.3.0` being cut, and three merged PRs in between, **When** the changelog feature runs, **Then** `CHANGELOG.md` gains a new section headed by `v1.3.0` (and a date) listing all three PRs by title, and every line that existed in the file before this run is preserved unchanged above the new section.
2. **Given** a repo with no `CHANGELOG.md` file yet, **When** the changelog feature runs for the first release, **Then** the file is created with a single dated section for that version — not an error.
3. **Given** a commit in range that has no associated PR (a direct push), **When** the changelog feature runs, **Then** that commit is still listed, using its raw commit subject line as a documented fallback (FR-009) — it is never silently dropped.
4. **Given** `paws semver --push` is run without also invoking the standalone `paws changelog` subcommand, **When** the release completes, **Then** behavior is identical to today — no `CHANGELOG.md` is touched.
5. **Given** the commit-back flag (FR-013) is enabled and the changelog write succeeds, **When** the commit is created, **Then** its message includes the documented loop-avoidance marker and the commit lands on the target repo's default branch, matching `mbround18/auto`'s current `Update CHANGELOG.md [skip ci]` behavior closely enough that `valheim-docker`'s existing `release.yml` trigger guard (which checks for that literal marker text) continues to work unmodified.

---

### User Story 3 - Full Docker tag matrix to retire `ghaction-docker-meta` outright (Priority: P1)

As a maintainer whose CI currently depends on `crazy-max/ghaction-docker-meta` for branch, PR, schedule, and sha tags in addition to the semver rollup, I want `paws docker` to optionally emit all of those tag types on builds where they apply, so I can delete the `ghaction-docker-meta` step from my workflow entirely instead of running `paws docker` alongside it for the tags it doesn't cover.

**Why this priority**: without this, User Story 1 only gets a consumer halfway — `docker-release.yml`'s full tag matrix and `docker-build.yml`'s branch/PR canary-tag flow both still need `ghaction-docker-meta` for the tag types this story adds, so the action can't actually be deleted. Grouped at P1 with User Story 1 since both are required for the same "drop the action" outcome.

**Independent Test**: run `paws docker` with the new matrix flags enabled against fixtures for each trigger shape (branch push, PR, schedule, tag) and assert the tag set for each matches the corresponding subset of `ghaction-docker-meta`'s configured `tags:` block in `valheim-docker`'s `docker-release.yml` (schedule/ref-branch/ref-pr/semver×3/sha), modulo the documented string-format differences in Out of Scope.

**Acceptance Scenarios**:

1. **Given** a branch-push build (`git_ref` = `refs/heads/some-branch`) with the branch-tag flag enabled, **When** tags are computed, **Then** output includes a tag derived from the branch name, mirrored across registries like existing tags.
2. **Given** a pull-request build with the PR-tag flag enabled, **When** tags are computed, **Then** output includes a `pr-{number}`-shaped tag, mirrored across registries.
3. **Given** a scheduled-trigger build with the schedule-tag flag enabled, **When** tags are computed, **Then** output includes a `schedule`-shaped tag (exact string TBD in plan.md), mirrored across registries.
4. **Given** any build with the sha-tag flag enabled, **When** tags are computed, **Then** output includes a `sha`-prefixed tag unconditionally, alongside whatever other tags the build already produces (not exclusively when no other tag type applies, unlike today's fallback-only `is_git_sha` behavior).
5. **Given** a release-tag build with rollup, sha, and `--with-latest` all enabled, **When** tags are computed, **Then** the full set (`{version}`, `{major}.{minor}`, `{major}`, `latest`, `sha-{sha}`) is produced with no duplicates and no cross-type interference — verifying the emit-all-applicable restructuring (see Affected Contracts) didn't regress any single tag type's own logic.

### Edge Cases

- What happens when `--tag-rollup` is used with a version string that doesn't parse as semver at all (e.g. a bare git sha, per `is_git_sha`)? → No rollup tags are produced; only the existing `sha-`-prefixed tag, since there is no major/minor to roll up from a sha.
- What happens when two rollup tags collide with an existing, unrelated tag already pushed by a different mechanism (e.g. someone manually pushed `image:3` by hand before ever using `paws docker`)? → Out of scope for this spec to prevent (registries always allow tag overwrite); note in docs that rollup tags are moving pointers, same caveat `ghaction-docker-meta`'s own docs carry today.
- What happens when the changelog feature runs on a `--prefix`-scoped tag range (e.g. a monorepo cutting `chart-name-v1.2.0`)? → Commit/PR range resolution must use the same prefix-aware last-tag lookup `paws semver` already implements (`resolve_last_tag`), not a bare "most recent tag" lookup that could cross an unrelated component's releases.
- What happens when the GitHub token used for changelog PR lookups lacks access to closed/merged PR data (rare, but possible with a narrowly-scoped token)? → Falls back to the same raw-commit-subject path as "no associated PR found" (FR-009) rather than failing the whole changelog generation.
- What happens when `paws docker --tag-rollup` is run for a `0.x` version (e.g. `v0.4.1`)? → Still emits `0` and `0.4` rollups — semver's own convention treats `0.x` as its own valid major line; no special-casing needed.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: `paws docker` MUST expose an opt-in flag (default off) that, when set, causes tag generation to additionally produce `major` and `major.minor` rollup tags for any version `generate_tags` already classifies as a release version via its existing `is_release_version` check (`git_ref` starts with `refs/tags/` **and** the version is not a prerelease per `is_prerelease_version`).
- **FR-002**: Rollup tags MUST NOT be produced for prerelease versions or for versions that fail semver parsing (including bare git-sha versions per `is_git_sha`) — only true `MAJOR.MINOR.PATCH` release versions produce rollups.
- **FR-003**: Rollup tags MUST be mirrored across every configured `--registries` entry using the exact same per-tag mirroring loop `generate_tags` already applies to the full-version and `latest` tags — no separate/divergent mirroring logic for rollup tags.
- **FR-004**: Rollup tags MUST respect existing `--target`/`--prepend-target` prefixing behavior identically to how the full-version tag does today (e.g. `odin-3`, `odin-3.2` when a target prefix is active).
- **FR-005**: Default `paws docker` behavior (flag omitted) MUST be byte-identical to today's `generate_tags` output — this is an additive, backward-compatible change per the constitution's Development Workflow section.
- **FR-006**: The rollup feature MUST ship a unit test asserting no duplicate tags are produced when `--with-latest` and the rollup flag are both enabled on the same release version.
- **FR-007**: `paws` MUST expose a new `paws changelog` subcommand (see Clarifications, Session 2026-08-23) that accepts a previous ref, a new version, and a target `CHANGELOG.md` path, and produces a new dated section listing every commit/PR in that range. It MUST be independently invocable (e.g. to preview an entry before a release), not gated behind `paws semver --push`.
- **FR-008**: Changelog generation MUST append a new section rather than rewriting the file — all pre-existing file content MUST be preserved byte-for-byte above the newly appended section. This is a new, `paws`-native changelog format; it is explicitly not a byte-for-byte port of `mbround18/auto`'s format (per Constitution Principle IV, this spec does not claim parity with a source that isn't `gh-reusable`).
- **FR-009**: For any commit in the changelog range that has no associated merged PR (i.e. the same "no PR found" case `paws-semver::fetch_pr_labels_for_commit` already has to handle for direct-push builds), changelog generation MUST fall back to that commit's raw subject line rather than omitting the commit.
- **FR-010**: Changelog generation MUST use the same prefix-aware last-tag resolution `paws-semver::resolve_last_tag` already implements when determining the "previous" end of the commit range, so multi-component/monorepo tag prefixes (e.g. `chart-name-v1.2.0`) don't pull in an unrelated component's commit history.
- **FR-011**: Changelog generation MUST be opt-in — `paws semver --push` (or any other existing entrypoint) with no changelog flag/subcommand invoked MUST NOT write or modify `CHANGELOG.md`, matching today's behavior exactly.
- **FR-012**: Both features (tag rollup, changelog) MUST ship crate-level unit tests per Constitution Principle V — no subcommand path merges with an `(unimplemented)` stub.
- **FR-013**: Changelog generation MUST expose an opt-in commit-back mode that, when enabled, commits the updated `CHANGELOG.md` to the target repo's default branch. The commit message MUST unconditionally include a documented loop-avoidance marker (matching `valheim-docker`'s existing `[skip ci]` text so its current `release.yml` trigger guard keeps working unmodified) — the marker is not itself a separate opt-in; enabling commit-back always includes it. Commit-back MUST be opt-in and off by default, matching FR-011's default-no-write behavior. If the push is rejected (e.g. the default branch moved since the commit was prepared), commit-back MUST fail loudly with a non-zero exit and a clear error — no automatic retry — leaving the generated entry text on stdout (per Affected Contracts) as the caller's fallback to apply manually (see Clarifications, Session 2026-08-23).
- **FR-014**: `paws docker` MUST expose opt-in flags to additionally emit, per applicable build trigger: a branch-ref tag (branch-push builds), a PR-ref tag (pull-request builds), and a `schedule` tag (scheduled-trigger builds) — each emitted alongside whatever other tags a given build already produces (rollup, sha, `latest`), not in place of them, and each mirrored through the same per-tag/per-registry loop `generate_tags` already applies to existing tags (no separate mirroring implementation per Risks).
- **FR-015**: `paws docker` MUST expose an opt-in flag to unconditionally include a `sha`-prefixed tag alongside other computed tags, rather than only as the fallback primary tag when no version/ref-based tag applies (today's `is_git_sha` behavior remains as the *default*, unflagged fallback; FR-015 is additive to it).
- **FR-016**: Major/minor extraction for rollup tags (FR-001) MUST use an actual semver parse (not string-splitting) of the release version; a version that fails to parse as valid `MAJOR.MINOR.PATCH` (including build-metadata-suffixed versions) MUST produce no rollup tags rather than a malformed or partial one.
- **FR-017**: Changelog commit/PR-range enumeration MUST go through a `HistoryProvider` abstraction rather than a hardcoded GitHub call — this spec ships one implementation (GitHub, via its compare/commits REST API) but the abstraction MUST be shaped so a future GitLab or local-`git log` provider can be added without changing `paws changelog`'s CLI contract (FR-007) or the `Changelog Entry`/`Commit Range` data shapes (Key Entities).
- **FR-018**: `HistoryProvider` selection MUST be automatic, based on environment (e.g. which platform-specific environment variables/token shape are present), not a new CLI flag the caller has to set — mirroring `paws`'s existing "no secrets on the command line, read from environment" constraint rather than adding provider selection as another piece of command-line surface. When no provider's environment signature matches, `paws changelog` MUST fail with an explicit, actionable error rather than silently no-op or guess.

### Key Entities

- **Rollup Tag Set**: the additional `{major}` and `{major}.{minor}` tag strings derived from a release-version's existing full-version tag, subject to the same release/prerelease gate as `latest`, extracted via a semver parse (FR-016).
- **Tag Matrix**: the full set of simultaneously-applicable tag types a single `paws docker` invocation can now emit — version, rollup, `latest`, sha, branch-ref, PR-ref, schedule — each independently opt-in and each flowing through the same registry-mirroring path.
- **Changelog Entry**: one dated, version-headed section appended to `CHANGELOG.md`, containing one line per commit/PR in the resolved range, each rendered as a PR title (preferred) or raw commit subject (fallback per FR-009).
- **Commit Range**: the `(previous_ref, new_version_ref]` pair changelog generation walks, resolved via the same prefix-aware last-tag logic `paws-semver::resolve_last_tag` uses today.
- **Changelog Commit**: the optional (FR-013) git commit that writes the generated `CHANGELOG.md` back to the default branch, carrying the loop-avoidance marker in its message.
- **History Provider**: the abstraction (FR-017–FR-018) behind which commit/PR-range enumeration runs; auto-selected by environment. This spec ships a GitHub implementation only; the interface is designed for a future GitLab or local-`git log` implementation to be added without a `paws changelog` CLI change.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: 100% of `paws docker` invocations without the rollup flag produce output identical to pre-feature `generate_tags` — verified by a regression test running the full existing `paws-docker` test suite unmodified against the new code path.
- **SC-002**: 100% of rollup-enabled invocations on a prerelease or non-semver version produce zero rollup tags.
- **SC-003**: A fixture repo with an existing `CHANGELOG.md` from a prior (non-`paws`) release process can adopt the changelog feature and see its pre-existing content fully intact after the first `paws`-generated section is appended.
- **SC-004**: `mbround18/valheim-docker`'s documented `image:3` pinning pattern (from its own README) round-trips correctly: a `paws docker --tag-rollup` publish for `v3.2.1` produces a pullable `image:3` tag pointing at that same release.
- **SC-005**: `valheim-docker`'s `docker-release.yml` can be rewritten to use `paws docker` alone (all matrix flags enabled) with the `crazy-max/ghaction-docker-meta` step deleted, and the resulting tag set for each trigger shape (branch/PR/schedule/tag) is a functional superset of what that workflow's current tag config produces (per FR-014/FR-015, modulo the declared string-format differences in Out of Scope) — this is the concrete "action fully retired" bar, not just "tags exist."
- **SC-006**: `valheim-docker`'s `release.yml` can be rewritten to chain `paws semver --push` with the standalone `paws changelog --commit` (or equivalent flag name, per plan.md) with the `mbround18/auto` step deleted, and the resulting commit history shows a `CHANGELOG.md`-updating commit with the loop-avoidance marker that does not re-trigger the release workflow on push — this is the concrete "action fully retired" bar for the changelog half.

## Assumptions

- `dagger`-mediated registry pushes (the actual `docker push` calls) are unaffected by this feature — it only changes which tag strings are computed and handed to the existing push path.
- **Resolved** (see Clarifications, Session 2026-08-23): commit/PR-range enumeration goes through a new `HistoryProvider` abstraction (FR-017–FR-018) rather than being hardcoded to one API. This spec ships exactly one implementation (GitHub's compare/commits REST API, consistent with `paws-semver`'s existing access pattern) but the abstraction and its auto-selection mechanism are designed up front so a GitLab provider or a local-`git log` provider can be added later without changing `paws changelog`'s CLI contract or FR-007's shape.
- Now that both actions are targeted for full retirement (see Summary), `ghaction-docker-meta`'s full tag matrix (branch/PR/schedule tags) is **in scope** via FR-014/FR-015/User Story 3 — this supersedes the prior assumption that it was a follow-up feature.
- The changelog feature reuses `paws-semver`'s existing GitHub API access pattern (token from environment, never a CLI flag, per the constitution's "No secrets on the command line" constraint) rather than introducing a second HTTP client/auth path.
- This spec is scoped to what unblocks `mbround18/valheim-docker`'s specific migration, but is written so the full-matrix/full-changelog behavior is generally useful to any consumer wanting to drop both actions, not `valheim-docker`-specific in its implementation.

## Validation Plan

- Unit tests in `paws-docker` for: rollup tag generation on a release version, no-rollup on prerelease, no-rollup on a bare-sha version, no-rollup on a version that fails semver parsing (FR-016), rollup + `with_latest` de-duplication, rollup + target-prefix interaction, rollup + multi-registry mirroring, branch-ref tag generation, PR-ref tag generation, schedule tag generation, unconditional sha-tag generation alongside other tag types, and a full-matrix combination test (rollup + sha + latest + no duplicates, per User Story 3 Acceptance Scenario 5).
- Unit tests in the new changelog module/crate for: append-only behavior against a pre-populated fixture `CHANGELOG.md`, first-run file creation, PR-title rendering, raw-commit-subject fallback (no PR found), prefix-scoped range resolution in a simulated monorepo fixture, commit-back behavior (FR-013): commit message contains the loop-avoidance marker, commit lands on the default branch, commit-back is a no-op when the flag is unset, a rejected push fails loudly with no retry and leaves the entry text on stdout — and `HistoryProvider` auto-selection (FR-018): GitHub environment signature selects the GitHub provider, no matching signature produces the documented explicit error (not a silent no-op), and the GitHub provider implementation is tested against the trait/interface in isolation from `paws changelog`'s CLI layer (so a future provider can be tested the same way).
- An end-to-end dry run against a fixture clone of `mbround18/valheim-docker`'s actual tag history and actual `CHANGELOG.md` (already present in that repo, `mbround18/auto`-formatted) validating SC-004, SC-005, and SC-006 concretely, not just against synthetic fixtures.
- `cargo test --workspace` continues to pass with zero failures (Constitution Principle V / SC-002 of `001-paws-core-cli`), and no existing `paws-docker` or `paws-semver` test's expected output changes as a side effect of this feature.

## Rollout and Rollback

- Ship both features as opt-in (new flags on `paws docker`, and the new standalone `paws changelog` subcommand simply not being invoked) so existing `paws` consumers see zero behavior change on upgrade — no coordinated migration is required to adopt the new `paws` version itself.
- `valheim-docker`'s own migration (separate, downstream work) can then adopt `--tag-rollup` and `paws changelog` incrementally: rollup tags first (higher priority, P1), changelog second (P2) — matching this spec's own story priorities.
- If a regression is found in either feature post-release, disable it consumer-side by dropping the opt-in flag or no longer invoking `paws changelog` — no `paws` rollback is required since default behavior is unchanged.
- If the changelog feature's format proves unworkable for a given consumer, they can stop invoking it at any release without losing prior entries — FR-008's append-only guarantee means no destructive migration is needed to abandon the feature.
