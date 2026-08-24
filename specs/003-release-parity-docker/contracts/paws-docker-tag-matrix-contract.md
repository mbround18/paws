# Contract: `paws docker` Tag Matrix

## 1) CLI flag contract (`paws-cli-core::DockerArgs`)

New flags, all opt-in (default `false`/unset), additive to the existing `DockerArgs` shape:

| Flag | Type | Gate (when it actually produces a tag) |
|---|---|---|
| `--tag-rollup` | bool | `is_release_version` (existing gate) **and** version parses as semver (FR-016) |
| `--tag-sha` | bool | always, when set (FR-015) — independent of `Version`'s own sha fallback |
| `--tag-branch` | bool | `event_name` implies a branch-push build (not a tag, not `pull_request`, not `schedule`) |
| `--tag-pr` | bool | `event_name == "pull_request"` **and** a PR number parses out of `git_ref` (R5) |
| `--tag-schedule` | bool | `event_name == "schedule"` |

Omitting all five flags MUST produce output byte-identical to today's `generate_tags` (FR-005,
SC-001) — this is the contract's non-negotiable backward-compatibility floor.

## 2) `paws-docker::generate_tags` contract

- Public signature: additive only. Either (a) new `Option`/bool parameters with `false`/`None`
  defaults matching every existing call site's behavior unchanged, or (b) a new higher-level
  function (`generate_tag_matrix` or similar) that existing callers never have to touch, with
  `generate_tags` itself untouched as a public symbol. Exact choice is an implementation detail
  for tasks.md; either satisfies this contract as long as FR-005 holds.
- Internal: builds a `Vec<TagKind>` (see data-model.md) before mirroring, so every tag type -
  existing and new - flows through the *one* per-tag/per-registry mirroring loop that exists
  today. No second mirroring implementation for new tag kinds (Risks: "duplicated tag-mirroring
  logic" risk).
- Rollup major/minor extraction goes through `semver::Version::parse` (FR-016) — a version that
  fails to parse produces zero rollup tags, not a partial/malformed one.

## 3) Registry-mirroring contract

Unchanged: every tag type introduced by this feature (rollup, sha, branch-ref, PR-ref, schedule)
is mirrored into every `--registries` entry using the same tag-value-after-the-colon substitution
`generate_tags` already performs for `version_tag`/`latest`. No tag type gets a bespoke mirroring
path.

## 4) Compatibility contract with `valheim-docker`'s current `ghaction-docker-meta` config

| `ghaction-docker-meta` `type=` | `paws docker` equivalent | Byte-identical string? |
|---|---|---|
| `semver,pattern={{version}}` | existing `version_tag` (unchanged) | Yes (already shipped) |
| `semver,pattern={{major}}.{{minor}}` | `--tag-rollup`'s `RollupMinor` | No — format decided by this feature, not a `ghaction-docker-meta` clone (Out of Scope) |
| `semver,pattern={{major}}` | `--tag-rollup`'s `RollupMajor` | No, same reason |
| `ref,event=branch` | `--tag-branch`'s `BranchRef` | No |
| `ref,event=pr` | `--tag-pr`'s `PrRef` (`pr-{number}`) | No |
| `schedule` | `--tag-schedule`'s `Schedule` | No |
| `sha` | `--tag-sha`'s `Sha` (`sha-{sha}`, existing prefix convention kept — see spec Out of Scope) | No (prefix differs) |

"No" here means: a tag of that *type* is produced, satisfying SC-005's functional-superset bar,
but the exact string may differ from `ghaction-docker-meta`'s own output — declared, not hidden,
per the spec's Out of Scope section.
