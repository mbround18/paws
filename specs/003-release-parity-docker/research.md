# Phase 0 Research: Full Docker Tag Matrix and In-Repo Changelog

Source spec: `specs/003-release-parity-docker/spec.md`. All three of the spec's Clarifications
(Session 2026-08-23) are resolved; this document resolves the *implementation-level* unknowns
plan.md needs before Phase 1 design — mainly "what existing `paws` code already does 80% of
this and should be reused instead of rebuilt."

## R1: Does `paws` already have a "detect which CI/git provider we're running under" mechanism?

**Decision**: Yes — `paws_environment::Provider` + `CiContext::detect()`
(`crates/paws-environment/src/lib.rs:158-204`). `paws changelog`'s `HistoryProvider` selection
(FR-018) reuses this exact mechanism rather than inventing a second one.

**Rationale**: `CiContext::detect()` already does precisely what FR-018 asks for: it checks
`$GITHUB_REPOSITORY` and returns a `Provider::GitHub` context (owner/repo/sha/ref/token) on a
match, and its own doc comment says outright: *"GitHub Actions only for now ... other providers
add another `if let Ok(..) = std::env::var(..)` branch here."* That is the exact shape FR-018
asks for (environment-based auto-selection, no CLI flag, explicit failure when nothing matches
— `CiContext::detect()` already `bail!`s with a clear message when no provider is found). Adding
a second, `paws-changelog`-local detection function would duplicate this and risk drifting from
it (e.g. a future GitLab CI addition landing in `paws_environment` but not in a
`paws-changelog`-local copy).

**Design consequence**: `HistoryProvider` (the trait, FR-017) is a `paws-changelog`-crate
concept — *what changelog generation needs from a history source* — but *which* provider to
construct is decided by matching on `paws_environment::CiContext::detect()`'s resolved
`Provider`, not by a new enum. `paws-changelog` depends on `paws-environment` for this; it does
not depend on `paws-environment` depending back on it (no cycle — `paws-environment` doesn't
need to know changelog generation exists).

**Alternatives considered**:
- A `paws-changelog`-local `HistoryProviderKind` enum with its own env-var detection: rejected
  — duplicates `CiContext::detect()`'s exact logic for no benefit, and two detection code paths
  is the kind of "second HTTP client/auth path" the spec's own Assumptions section already rules
  out for the token-resolution half of this problem.
- Detecting provider from git remote URL (`git remote get-url origin` matching `github.com` vs
  `gitlab.com`): rejected — `paws` has no local-git dependency anywhere today (see R2), and this
  would introduce one just for detection when the environment-variable approach already works
  and matches the constitution's env-var-first posture.

## R2: How does `paws changelog` enumerate commits/PRs in `(previous_ref, new_version_ref]`?

**Decision**: GitHub's REST "compare two commits" endpoint
(`GET /repos/{owner}/{repo}/compare/{base}...{head}`), which returns the exact commit list
between two refs in one call, each with its own `sha`/`commit.message`. This is the
`GitHubHistoryProvider` (the spec's sole shipped implementation, FR-017) content.

**Rationale**: matches Clarifications Session 2026-08-23's second resolution (a remote API, not
local git) and reuses `paws-release`'s existing `reqwest` + GitHub REST-API-with-bearer-auth
pattern (`crates/paws-release/src/lib.rs`'s `GitHubReleaseClient`) rather than adding a GraphQL
call alongside `paws-semver`'s existing GraphQL tag-lookup path. The compare endpoint gives commit
SHAs directly, which then feed the existing per-commit PR-lookup call (R3) — no separate
"list commits" call needed beyond the one compare request.

**Alternatives considered**:
- GitHub's GraphQl API (matching `paws-semver::GitHubGraphQlTagSource`'s existing transport):
  rejected for this specific call — the compare endpoint is REST-only; there is no GraphQL
  equivalent for "commits between two refs" as a single call, and mixing REST-for-compare with
  GraphQL-for-tags in the same crate would add a second HTTP client shape for no benefit.
- Local `git log --oneline base..head`: rejected per the spec's Clarifications resolution
  (requires a full-history checkout `paws` can't verify, and `paws` has zero local-git-history
  code today — see R1's alternatives).

## R3: How does `paws changelog` get a PR title (not just labels) for a commit?

**Decision**: New function in `paws-changelog`'s GitHub provider, hitting the same
`GET /repos/{owner}/{repo}/commits/{sha}/pulls` endpoint `paws-semver::fetch_pr_labels_for_commit`
already calls (`crates/paws-semver/src/lib.rs:216-257`), reading `.first().title` instead of
`.first().labels`. Same best-effort semantics: empty/`None` (not an error) when no PR is
associated, which is exactly FR-009's "no PR found → raw commit subject fallback" trigger.

**Rationale**: `fetch_pr_labels_for_commit`'s doc comment already establishes the precedent this
needs — a merge-to-main `push` event has no `github.event.pull_request` context, so this lookup
is the only way to recover "which PR did this commit come from" after the fact. It only reads
`.first()` (one PR per commit), which the codebase's own comment on that function and this
research's fixture check (see quickstart.md) both confirm is safe for a squash-merge workflow
like `valheim-docker`'s (`fix/mod support (#1468)`-style single-commit-per-PR history).

**Alternatives considered**: importing `fetch_pr_labels_for_commit` itself and ignoring its
labels: rejected — it fetches a different field (`labels`, not `title`); a title-only sibling
function is one HTTP call shape reused with a different response-field read, not a new pattern.

## R4: How does `paws changelog --commit` write `CHANGELOG.md` back to the default branch?

**Decision**: `paws_release::GitHubReleaseClient::get_content` / `put_content`
(`crates/paws-release/src/lib.rs:620-703`), the exact mechanism `paws llms generate --publish`
already uses end-to-end (`run_llms_generate`, `crates/paws-cli-core/src/lib.rs:2025-2086`).

**Rationale**: this is a complete, already-tested match for FR-013's requirements:
- `get_content` returns `None` on a 404 (no existing file) — directly satisfies User Story 2
  Acceptance Scenario 2 ("no `CHANGELOG.md` file yet ... created ... not an error").
- `put_content` takes an optional prior `sha`; passing the wrong one (stale/omitted-when-it
  shouldn't-be) makes GitHub's Contents API itself reject with 409/422, which `put_content`
  already surfaces as a **loud `anyhow::bail!` with status+body, no retry** — this is *exactly*
  FR-013's "fail loudly, no automatic retry" resolution (Clarifications Session 2026-08-23) with
  zero new code; the existing function already behaves this way.
- `run_llms_generate`'s `should_publish` helper plus its literal `"chore: regenerate llms.txt
  [skip ci]"` commit message is the established, already-shipped precedent for the loop-avoidance
  marker FR-013 requires — `paws changelog --commit`'s commit message follows the identical
  `[skip ci]` convention (matching `valheim-docker`'s own existing `Update CHANGELOG.md [skip
  ci]` message, confirmed from its commit history during spec review).

**Design consequence**: `paws-changelog` depends on `paws-release` for `GitHubReleaseClient`
rather than rolling its own Contents-API client — the same dependency `paws-cli-core` already
has, just used one crate lower in the stack.

**Alternatives considered**: a local `git commit`/`git push` (shelling out or via a git library):
rejected — `paws` has no local-git-mutation code anywhere today (Contents-API-over-HTTP is the
established pattern for every existing "commit a generated file" flow: `llms.txt`, Helm's
`index.yaml`), and introducing one now would be a second, inconsistent mechanism for the same
kind of operation `paws` already solved.

## R5: How does `paws docker` get a PR number for a PR-ref tag (FR-014) without a new required input?

**Decision**: parse it out of the existing `git_ref` input. GitHub Actions sets `GITHUB_REF` to
`refs/pull/{number}/merge` on a `pull_request` event — `paws-docker`'s `GithubContext.git_ref`
already carries this string today (used for the `refs/tags/`/`refs/heads/` checks in
`should_push_image` and `is_release_version`). PR-ref tag generation adds one more parse of the
same field: match `refs/pull/(\d+)/`, capture the number.

**Rationale**: avoids adding a new required `--pr-number` CLI flag (which would violate FR-014's
"opt-in flags only, no new required input" framing implicit in FR-005's backward-compatibility
bar) and matches how `event_name`/`git_ref` are already the sole GitHub-context inputs
`resolve_docker_facts` takes — no new `DockerFactsInput`/`GithubContext` field needed, just a new
parse of an existing one.

**Alternatives considered**: a new `--pr-number` flag, falling back to `$GITHUB_EVENT_PATH`
JSON parsing: rejected — `paws-docker` reads no JSON event payload anywhere today, and the ref
string already contains the number for the one CI provider `paws` supports.

## R6: Does `paws-docker` need a new dependency for FR-016's semver parse?

**Decision**: yes — add `semver.workspace = true` to `crates/paws-docker/Cargo.toml` (the
workspace already pins `semver = "1"` for `paws-semver`; this is a version-shared addition, not a
new external dependency to the workspace).

**Rationale**: confirmed by reading `crates/paws-docker/src/lib.rs` — `is_prerelease_version` is
a substring check (`["alpha","beta","rc","dev"]`), not a semver parse, and `Cargo.toml` has no
`semver` dependency today. FR-016 explicitly requires an actual parse (not string-splitting) for
major/minor extraction, so this is a real, named new dependency — not an oversight to catch
later in review.

**Design consequence**: `is_prerelease_version`'s substring check is **not replaced** — FR-002
still gates rollups on the existing `is_release_version` (which uses `is_prerelease_version`)
exactly as today; the new `semver::Version::parse` call is *only* used for major/minor
*extraction* on versions that already passed that existing gate, one parse, one purpose. Keeps
FR-005's byte-identical-default guarantee intact (nothing about today's gating logic changes).

## Summary of new crate/dependency surface

| Crate | Change |
|---|---|
| `paws-docker` | +`semver` dependency (R6); `generate_tags` internal restructuring (emit-all-applicable, see data-model.md) |
| `paws-changelog` (new) | depends on `paws-environment` (R1), `paws-release` (R4), `paws-semver` (`resolve_last_tag` reuse, FR-010), `reqwest`/`serde_json`/`anyhow`/`tokio`/`schemars` (matching sibling crates), `async-trait` (the `HistoryProvider` trait is used as a trait object for provider auto-selection, so it needs dyn-safe async methods — native async-fn-in-traits isn't dyn-compatible). No date/time crate (`chrono` etc.) — `ChangelogEntry.date` is a manually formatted `String` (see data-model.md, analysis finding U1) |
| `paws-cli-core` | new `Commands::Changelog(ChangelogArgs)`; `DockerArgs` gains new opt-in flags |
| `paws-core` | `PipelineDefaults` gains a `changelog_path: Option<String>` field (default `CHANGELOG.md`), per the constitution's "shared defaults live in one place" |
