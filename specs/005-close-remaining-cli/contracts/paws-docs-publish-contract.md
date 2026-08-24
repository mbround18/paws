# Contract: `paws docs --provider`

## 1) CLI flag contract (`paws-cli-core::DocsArgs`)

| Flag | Type | Notes |
|---|---|---|
| `--provider <name>[,<name>...]` | comma-delimited `Vec<String>`, no default | each value parsed to `PublishTarget`; an unrecognized value is a normal clap "invalid value" error (Edge Cases) |
| `--repository` | `Option<String>` | falls back to `$GITHUB_REPOSITORY`, mirrors `HelmArgs`/`GenerateArgs` |
| `--branch` | `String`, default `"main"` | mirrors `HelmArgs::pages_branch`'s default shape |

Omitting `--provider` MUST reproduce today's exact behavior (FR-002): local `cargo doc --workspace
--no-deps` build only, nothing published.

## 2) `PublishTarget` value contract

| Value | Behavior |
|---|---|
| `github-pages` | Implemented — builds once, publishes via the auto-selected mechanism (§3) |
| `cloudflare-pages` | Recognized, immediately fails with "not implemented yet — see docs/ROADMAP.md" (FR-004a) — no build/publish attempt |
| `s3` | Same as `cloudflare-pages` |
| anything else | Clap parse error before any work starts |

## 3) `github-pages` provider mechanism-selection contract

1. Query `GET /repos/{owner}/{repo}/pages` (`GitHubReleaseClient::get_pages_config`).
2. `404` (not configured) or `build_type == "legacy"` → Git Trees API bulk commit.
3. `build_type == "workflow"` → Pages deployment API, **gated on** `$ACTIONS_RUNTIME_TOKEN`/
   `$ACTIONS_RESULTS_URL` being present (research.md R5); absent → fail with an error explicitly
   naming those env vars, no attempted deployment call.

Never a per-file `put_content` loop (FR-003) — the Git Trees path is one blob-create per file
(no commit, no rate-limit-relevant event) followed by exactly one tree/commit/ref-update sequence
for the whole set.

## 4) Multi-provider execution contract (FR-002a)

- The `cargo doc` tree is built exactly once regardless of how many providers are named.
- Every named provider runs concurrently (`tokio::task::JoinSet`, mirroring
  `paws-provision::provision_with_timing`'s exact shape — research.md R8).
- Every provider's outcome (success, or a specific failure — including FR-004a's "not
  implemented" case) is included in the reported result; no provider's outcome is ever hidden by
  another's.
- The command exits non-zero if any named provider failed, regardless of how many succeeded.

## 5) Idempotency contract

A second `--provider github-pages` run against unchanged `cargo doc` output MUST be a safe no-op
(or a no-diff commit) — same bar `llms generate --publish`'s `should_publish` helper already
holds itself to (Edge Cases).
