# Contract: `paws changelog` Subcommand

## 1) CLI contract (`paws-cli-core::ChangelogArgs`, new)

Standalone subcommand (Clarifications, Session 2026-08-23) — `Commands::Changelog(ChangelogArgs)`,
new entry in `paws-cli-core`'s `Commands` enum alongside `Docker`/`Semver`/etc.

| Flag | Type | Notes |
|---|---|---|
| `--version` | required `String` | the version this entry is for (e.g. `v1.3.0`) |
| `--previous-ref` | `Option<String>` | overrides the auto-resolved previous tag; falls back to `paws-semver::resolve_last_tag`-style prefix-aware resolution (FR-010) when unset |
| `--prefix` | `Option<String>` | passed straight through to the same resolution `paws semver` uses, for monorepo tag prefixes |
| `--output` | `Option<String>` | target `CHANGELOG.md` path; falls back to `PipelineDefaults::changelog_path` (default `"CHANGELOG.md"`) |
| `--commit` | bool, default `false` | opt-in commit-back (FR-013) |
| `--repository` | `Option<String>` | `"owner/repo"` override, same pattern as `GenerateArgs::repository` in `run_llms_generate`; falls back to `CiContext::detect()` |
| `--branch` | `Option<String>` | branch to commit to when `--commit` is set; falls back to the detected default branch, same pattern as `paws docker --default-branch` |

No secret-bearing flag exists — the GitHub token is resolved exclusively via
`paws_environment::resolve_github_token`/`CiContext::detect()`, matching the constitution's
"no secrets on the command line" constraint (Runtime and Defaults Impact).

## 2) Output contract

- Always (regardless of `--commit`): the rendered `ChangelogEntry` section text is printed to
  stdout, and the local file at `--output` is updated in place (append-only, FR-008).
- With `--commit`: additionally, a git commit is created against `--repository`'s `--branch` via
  `GitHubReleaseClient::put_content`, with a commit message containing the loop-avoidance marker
  (literal `[skip ci]`, matching `valheim-docker`'s own existing convention).
- Exit code: non-zero on any failure, including a rejected commit-back push (FR-013;
  Clarifications Session 2026-08-23) — no automatic retry. The already-printed stdout entry text
  is the documented manual-recovery path.

## 3) `HistoryProvider` contract (internal, `paws-changelog`)

- `commits_in_range(base, head)` returns every commit in `(base, head]`; never partial-and-silent
  — a failure to reach the provider is a hard error, not an empty-list fallback (that fallback is
  reserved for the narrower "no PR found for this commit" case, FR-009).
- `pr_title_for_commit(sha)` returns `Ok(None)` (not an error) when no PR is associated with the
  commit, or when the lookup fails for a reason FR-009 already classifies as "fall back" (e.g. a
  narrowly-scoped token per spec Edge Cases) — callers render `ChangelogLine::RawCommit` in that
  case, never omitting the commit (FR-009).
- Provider selection (FR-018) is automatic via `paws_environment::CiContext::detect()` (research.md
  R1) — `paws changelog` never exposes a `--provider` flag. No matching environment signature is a
  hard error with an actionable message (mirrors `CiContext::detect()`'s existing `bail!` shape).

## 4) `CHANGELOG.md` file contract

- Append-only: every byte that existed in the file before a run is preserved, unchanged, above the
  newly appended section (FR-008; SC-003).
- First-run (file doesn't exist): created fresh with a single section, not an error (User Story 2
  Acceptance Scenario 2).
- Section content: one dated, version-headed Markdown section per run, one line per commit/PR in
  range, each either a PR title or (FR-009 fallback) a raw commit subject — exact Markdown
  structure is a `paws`-native format (Out of Scope: not a byte-for-byte `mbround18/auto` clone).

## 5) Compatibility contract with `valheim-docker`'s current `mbround18/auto` flow

| `mbround18/auto` behavior | `paws changelog` equivalent |
|---|---|
| Runs inside `release.yml`'s `Release` job via a dedicated Action step | `paws changelog --commit` invoked as a step after `paws semver --push` in the migrated workflow (downstream work, not this spec) |
| Commits `CHANGELOG.md` to `main` as `Update CHANGELOG.md [skip ci]` | Commits via `GitHubReleaseClient::put_content` with a `[skip ci]`-marked message (SC-006) |
| `release.yml`'s own `!contains(..., 'skip ci')` trigger guard prevents a loop | Unchanged — still the consumer's own workflow-YAML responsibility (Risks); `paws` only guarantees the marker is present |
| Markdown format: PR-title bullets, `#### <label-derived-category>` sub-headers | New `paws`-native format (FR-008) — not reproduced byte-for-byte (Out of Scope) |
