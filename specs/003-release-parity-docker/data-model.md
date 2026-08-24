# Data Model: Full Docker Tag Matrix and In-Repo Changelog

Entities correspond 1:1 to the spec's Key Entities section; this document adds field-level shape
and the internal restructuring `generate_tags` needs (Affected Contracts' "one non-additive
internal change").

## Docker tag matrix

### `TagKind` (new, internal to `paws-docker`)

Replaces `generate_tags`'s current "compute one `version_tag` string" logic with an ordered list
of applicable kinds, each independently gated. Not part of the public API — `generate_tags`'s
existing signature and default output stay byte-identical (FR-005); this is the internal shape
the restructuring (Affected Contracts) produces.

```rust
enum TagKind {
    Version(String),      // today's sole output — "v3.2.1" or "sha-<sha>"
    Latest,                // today's --with-latest, gated on is_release_version
    RollupMajor(String),   // "3" — FR-001/FR-002, gated on is_release_version + semver parse (FR-016)
    RollupMinor(String),   // "3.2" — same gate
    Sha(String),            // "sha-<sha>" unconditional — FR-015, independent of Version's own sha fallback
    BranchRef(String),      // branch name, sanitized — FR-014
    PrRef(u64),              // PR number parsed from git_ref — FR-014, R5
    Schedule,                 // FR-014, exact string TBD (see Open Item below)
}
```

Every `TagKind` flows through the *same* target-prefix + registry-mirroring loop
`generate_tags` already applies to today's `version_tag`/`latest` (FR-003/FR-014's "no separate
mirroring implementation" requirement) — the restructuring is: build the `Vec<TagKind>` first
(today's version_tag + latest, plus whichever new kinds their opt-in flag and gate allow), *then*
run the existing per-tag mirroring loop over the resulting tag strings once, unchanged.

**Validation rules**:
- `RollupMajor`/`RollupMinor` only appear when `is_release_version` is true AND the version parses
  via `semver::Version::parse` (FR-002, FR-016) — a non-parsing release version produces neither.
- `Sha` only appears when its opt-in flag is set — independent of whether `Version` itself already
  resolved to a `sha-`-prefixed tag (today's `is_git_sha` fallback); both can coexist without
  duplication because they're different `TagKind` variants even if their string output happens to
  collide (dedup happens at the final string-set level, matching FR-006's existing dedup contract
  for `--with-latest` + rollup).
- `BranchRef`/`PrRef`/`Schedule` are only constructed when `event_name`/`git_ref` actually match
  their trigger shape (branch push / `pull_request` / `schedule`) — the opt-in flag alone doesn't
  produce a tag on a build shape it doesn't apply to (spec Edge Cases: no forcing a PR tag onto a
  non-PR build).

**Open item for tasks.md**: the `Schedule` tag's exact string content (spec's User Story 3
Acceptance Scenario 3 marks this "TBD in plan.md"). Recommendation: literal `"schedule"`,
matching `ghaction-docker-meta`'s own `type=schedule` default output closely enough for
consumers migrating off it, with no timestamp/nightly-date suffix (keeps the tag a stable,
overwritable pointer like `latest` rather than an ever-growing tag list) — confirm in tasks.md
before implementation since it's the one remaining string-format decision this plan doesn't lock.

### `DockerFactsInput`/`GithubContext` — unchanged shape, new derived reads

No new fields. `BranchRef`'s branch name and `PrRef`'s PR number are both derived by parsing the
*existing* `GithubContext.git_ref` field (R5) — `refs/heads/{branch}` and
`refs/pull/{number}/merge` respectively. `Schedule`'s gate reads the *existing*
`GithubContext.event_name` field (`== "schedule"`), the same field `should_push_image` already
branches on for `"pull_request"`.

## Changelog

### `HistoryProvider` (new trait, `paws-changelog`)

```rust
#[async_trait::async_trait]
trait HistoryProvider {
    /// Commits in (base, head], newest-first or oldest-first (impl's choice,
    /// documented) — each with enough to render a fallback line (FR-009).
    async fn commits_in_range(&self, base: &str, head: &str) -> Result<Vec<HistoryCommit>>;

    /// Best-effort: the title of the PR associated with `sha`, or `None`
    /// if there isn't one / the lookup fails for a reason FR-009 already
    /// treats as "fall back" (no access, no associated PR).
    async fn pr_title_for_commit(&self, sha: &str) -> Result<Option<String>>;
}

struct HistoryCommit {
    sha: String,
    subject: String,   // raw commit subject line — FR-009's fallback text
}
```

`GitHubHistoryProvider` is the sole implementer this spec ships (R2, R3), constructed from a
`paws_environment::CiContext` (R1) — `owner`/`repo`/`token` come straight from
`CiContext::detect()`'s resolved fields, no separate credential resolution path.

### `ChangelogEntry` (new struct, `paws-changelog`)

```rust
struct ChangelogEntry {
    version: String,       // e.g. "v1.3.0"
    date: String,           // ISO "YYYY-MM-DD", formatted manually (std::time) — no new date-crate dependency (resolved: analysis U1, avoids adding `chrono`, which no crate in this workspace depends on today)
    lines: Vec<ChangelogLine>,
}

enum ChangelogLine {
    PullRequest { title: String, sha: String },
    RawCommit { subject: String, sha: String },  // FR-009 fallback
}
```

Rendering (Markdown section header + bullet list) is a pure function of `ChangelogEntry` — no I/O
— so it's independently unit-testable from both the `HistoryProvider` call and the file-append
step (Validation Plan's "PR-title rendering" / "raw-commit-subject fallback" unit tests).

### `CommitRange` resolution

Reuses `paws_semver::resolve_last_tag` (unchanged signature) to get the *previous* tag/prefix
(FR-010) — `paws-changelog` takes the same `prefix: Option<String>` input `paws semver` does and
passes it straight through, so `paws changelog` run against a monorepo's `chart-name-v1.2.0`-style
tags resolves the same "previous" tag `paws semver` itself would compute for that prefix. The
*new* end of the range is always the caller-supplied `--version` (no auto-resolution needed there
— it's the version currently being released).

### `CHANGELOG.md` write path

Two independent steps, both idempotent given the same inputs:
1. **Local write** (always, FR-007/FR-008): read existing file (or treat as empty per User Story 2
   Acceptance Scenario 2), append the rendered `ChangelogEntry` section, write back.
2. **Commit-back** (opt-in, FR-013): `GitHubReleaseClient::get_content`/`put_content` against the
   *same* path and the target repo's default branch (R4) — independent of step 1's local write
   succeeding against a different working-directory path than the one committed, though in
   practice both target the same `--output` path.

### `paws-core::PipelineDefaults` addition

```rust
pub struct PipelineDefaults {
    pub toolchain: Option<String>,
    pub registry: Option<String>,
    #[serde(default)] // NEW — required so existing serialized payloads missing this key still deserialize
    pub changelog_path: Option<String>, // default "CHANGELOG.md" when unset
}
```

Additive field on an existing `#[derive(Serialize, Deserialize)]` struct. `serde`'s derive treats
a missing key as an error by default *regardless of the field's type* — `Option<T>` alone does
not make a field implicitly optional on deserialize — so `#[serde(default)]` is required here for
`crates/paws-core/src/lib.rs`'s existing round-trip test (and any already-serialized
`PipelineDefaults` JSON on disk) to keep parsing after this field is added.
