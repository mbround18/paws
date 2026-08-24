# Phase 0 Research: Close Remaining Gaps Found Migrating `valheim-docker`

Source spec: `specs/005-close-remaining-cli/spec.md`. All three of the spec's Clarifications
(Session 2026-08-23) are resolved; this document resolves the remaining implementation-level
unknowns for all four gaps, by reading the actual code this spec touches rather than guessing.

## R1 (Gap 2 — clippy): confirms the fix is a one-line, no-dependency change

**Decision**: add `"--", "-D", "warnings"` to the non-wasm branch's `cargo clippy` `push_exec` call
in `crates/paws-rust/src/lib.rs:160`, matching the wasm branch's existing
`&["cargo", "clippy", "--target", WASM_TARGET, "--", "-D", "warnings"]` (minus `--target`).

**Rationale**: confirmed by reading the exact line — no design decision needed here beyond what
the spec's FR-001 already states verbatim. Flagged separately only because plan.md's Constitution
Check treats it as a deliberate default-behavior change (Risks: "declared breaking change"), not
because the code change itself is ambiguous.

## R2 (Gap 4 — Rust dependency scanner): `cargo-audit`, not `cargo-deny`

**Decision**: `cargo-audit` (via its `cargo audit --json` output), not `cargo-deny`.

**Rationale**: FR-008 asks specifically for RustSec-advisory coverage — `cargo-audit` is a
purpose-built, zero-config match for exactly that (reads `Cargo.lock` against the RustSec
Advisory DB, no project-side config file required), the same "one tool, one job" shape
Semgrep (SAST) and Gitleaks (secrets) already establish in this catalog
(`crates/paws-audit/src/lib.rs`'s `AUDIT_SCANNER_REGISTRY`). `cargo-deny` is a superset
(licenses, banned crates, sources, *and* advisories) that requires a `deny.toml` a fresh
consumer repo won't have by default — adding it would either silently skip most of its own
checks (no config) or force a new required config file on every Rust consumer, neither of
which matches this catalog's existing zero-config-needed pattern.

**Design consequence**: the new `ScannerConfig` entry uses `ScannerFamily::Language(LanguageFamily::Rust)`
(`crates/paws-audit/src/lib.rs:114`) — this variant already exists in the enum but is unused by
Semgrep/Gitleaks today (both use `ScannerFamily::CrossLanguage`), confirming the spec's claim that
the catalog shape already has everything a language-specific scanner needs.

**Alternatives considered**: `cargo-deny` — rejected per above. Running both — rejected as
unnecessary scope for this spec; `cargo-deny` can be a genuine follow-up (tracked in
`docs/ROADMAP.md`) if license/ban-list coverage is ever asked for, distinct from RustSec advisory
coverage.

## R3 (Gap 4): output shape and container image

**Decision**: `cargo audit --json` output parses into the existing `AuditScannerResult`/`TopFinding`
shapes via a new `parse_cargo_audit_findings` function, mirroring `parse_semgrep_findings`'s exact
pattern (`crates/paws-audit/src/lib.rs:800`). Container image: `rust:1-bookworm` (the same tag
`paws-rust`'s own default pipeline already pulls) with `cargo install cargo-audit --locked` run
once as part of the scanner's own script (matching `scanner_script`'s existing per-scanner
`with-new-file` + `sh <path>` pattern — no new builder image needed, since `cargo-audit` doesn't
need a pre-baked toolchain image the way `004-rust-coverage`'s `cargo-llvm-cov` did; a scanner run
is infrequent enough that install-at-run-time is fine here, unlike the `--coverage` case which
runs the more performance-sensitive test/build path).

**`cargo audit --json` schema** (relevant subset): `{"vulnerabilities": {"list": [{"advisory":
{"id": "RUSTSEC-YYYY-NNNN", "title": ..., "description": ..., "severity": ...}, "package": {"name":
..., "version": ...}}]}}`. Each list entry maps to one `TopFinding`, matching how
`parse_semgrep_findings` maps each Semgrep result entry to one.

**should_run gating**: `applies_to: &[LanguageFamily::Rust]` in the new registry entry — reuses the
exact same `select_audit_scanners` logic (`crates/paws-audit/src/lib.rs:358`) every other scanner
already goes through; no new gating code needed, only a new registry row.

## R4 (Gap 1 — docs publish): `GitHubReleaseClient` needs three new methods, not a new client

**Decision**: extend the existing `GitHubReleaseClient` (`crates/paws-release/src/lib.rs`) with:
- `get_pages_config(&self) -> Result<Option<PagesConfig>>` — `GET /repos/{owner}/{repo}/pages`,
  `None` on 404 (Pages not configured yet), else `{ build_type: "legacy" | "workflow" }`.
- `create_blob(&self, content: &[u8]) -> Result<String>` — `POST .../git/blobs`, returns the blob
  `sha`.
- `publish_tree(&self, branch: &str, files: &[(String, String)], message: &str) -> Result<()>` —
  the Git Trees API bulk-commit sequence: `POST .../git/trees` (one tree entry per file, each
  referencing a blob `sha` from `create_blob`, `base_tree` set to the branch's current commit's
  tree so unrelated existing files on that branch are preserved), `POST .../git/commits` (new tree
  + current branch-tip `sha` as the single parent), `PATCH .../git/refs/heads/{branch}` (fast-forward
  to the new commit).

**Rationale**: `create_blob`+`publish_tree` is a fundamentally different operation from
`put_content` (per-file `PUT .../contents/{path}`, one commit per call) — blob creation doesn't
create a commit at all (so N blob calls don't risk N webhook/CI triggers or N secondary-rate-limit
strikes the way N `put_content` commits would), and the tree/commit/ref-update sequence is exactly
one commit for the whole file set, satisfying FR-003's "not a naive per-file loop" requirement
directly. `GitHubReleaseClient` already has `api_base()`/`auth_headers()` helpers these three new
methods reuse as-is — no new HTTP client, no new auth path.

**Design consequence**: `crates/paws-cli-core::run_docs`'s `github-pages`-provider path walks
`target/doc`'s files, blob-creates each (concurrently — no ordering dependency between blobs), then
calls `publish_tree` once with the full path→blob-sha list.

## R5 (Gap 1): the Pages "workflow" `build_type` can only run inside a real GitHub Actions job

**Decision**: when `get_pages_config` reports `build_type: "workflow"`, `paws docs --provider
github-pages` MUST check for `ACTIONS_RUNTIME_TOKEN`/`ACTIONS_RESULTS_URL` (the env vars GitHub
Actions' own artifact-upload mechanism requires) and fail with a specific, actionable error if
they're absent — not attempt the deployment API and get a confusing failure partway through.

**Rationale**: confirmed by how `actions/upload-pages-artifact`/`actions/deploy-pages` actually
work — GitHub's Pages *deployment* API (as opposed to the Pages *config* read in R4) requires
uploading a build as a GitHub Actions "artifact" first, and artifact upload is itself tied to the
Actions runtime (it isn't a generic authenticated-REST operation available to an arbitrary
machine/token the way Contents/Git-Trees calls are). A `paws docs --provider github-pages` run
outside an Actions job (e.g. a maintainer dogfooding locally, matching how this session validated
`004-rust-coverage`'s coverage feature by running `paws ci` directly) genuinely cannot use this
path — this is a real environmental constraint, not an implementation gap to route around.

**Design consequence**: this is the one case in this spec where a `PublishTarget` provider's
internal mechanism selection (Git Trees vs. Pages deployment, both *within* `github-pages`, per
Clarifications) can itself fail based on environment — distinct from `cloudflare-pages`/`s3`
failing unconditionally (FR-004a). The error message must name the missing env vars explicitly, matching
this codebase's existing convention for env-var-gated failures (e.g. `paws_environment::resolve_github_token`'s
error naming exactly which vars it checked).

## R6 (Gap 3 — build cache): `paws-dagger`'s subprocess already inherits `DAGGER_CLOUD_TOKEN`

**Decision**: confirmed by reading `crates/paws-dagger/src/lib.rs` — every `Command::new("dagger")`
call site has no `.env_clear()`/`.env()` override affecting inherited variables (the one `.env()`
call, line 105, sets `BIN_DIR` for the install script only). `tokio::process::Command` inherits the
full parent environment by default, so `DAGGER_CLOUD_TOKEN`, if present in `paws`'s own process
environment, already reaches the `dagger` subprocess with zero code change.

**Design consequence**: the `CacheBackend::DaggerCloud` provider (Clarifications, Session
2026-08-23) needs no new plumbing code at all — it's a detection-and-logging concern only (FR-006's
"independently verifiable" requirement): check whether `DAGGER_CLOUD_TOKEN` is set, log which
backend was selected, done. This resolves the spec's own Validation Plan item ("Confirm ... before
implementing FR-005, whether `Command::new("dagger")` already inherits `DAGGER_CLOUD_TOKEN`") —
answer: yes, confirmed by source inspection, not by assumption.

## R7 (Gap 3): GitHub Actions cache provider needs a new mechanism — `actions/cache`'s REST API

**Decision**: the `CacheBackend::GitHubActionsCache` provider wraps the local Dagger engine's
on-disk state directory (confirmed via `dagger` CLI's own documented cache location, resolved
concretely in tasks.md) using the GitHub Actions Cache REST API (`POST/GET
.../actions/caches` — the same API `actions/cache@vN` itself calls, callable directly over HTTP
with `ACTIONS_CACHE_URL`/`ACTIONS_RUNTIME_TOKEN`, both only present inside a real Actions job — same
environmental constraint as R5's Pages-deployment case). Detected by: `ACTIONS_CACHE_URL`
(or its newer `ACTIONS_RESULTS_URL`) present.

**Rationale**: this is genuinely new code to own (per the spec's own Risk framing), unlike R6's
near-zero-code `DaggerCloud` provider — restoring the engine's persisted state before a Dagger
pipeline runs, and saving it back after, bracketing the existing `paws_dagger::core`/`core_streaming`
call sites. Scoped narrowly: this provider only ever activates inside GitHub Actions (same
detection signature `paws_environment::CiContext::detect()` already uses for `Provider::GitHub`),
so it reuses that existing detection rather than inventing a third "are we in CI" check.

**Design consequence (precedence)**: per Clarifications, `DAGGER_CLOUD_TOKEN` wins when both
signatures match (a consumer with a Dagger Cloud subscription running inside GitHub Actions still
gets the Dagger-native backend, which is the more capable/purpose-built one) — `CacheBackend`
selection checks `DAGGER_CLOUD_TOKEN` first, `ACTIONS_CACHE_URL`/`ACTIONS_RESULTS_URL` second, else
no backend.

**Not everyone wants to rely on Dagger Cloud** — a real, explicit product constraint, not just an
implementation footnote: Dagger Cloud is a paid/hosted third-party service `DaggerCloud`
opts a consumer *into*, while `GitHubActionsCache` needs no external account at all (every GitHub
Actions job already has the scope `actions/cache`-style usage implies, for free). Precedence only
decides which backend wins when *both* signatures are present — it does not make `DaggerCloud`
the "primary" provider and `GitHubActionsCache` a fallback in build effort or test coverage.
`GitHubActionsCache` is the provider the overwhelming majority of GitHub Actions-only consumers
(no Dagger Cloud subscription) will actually depend on, so it gets the same implementation rigor
as R6's near-zero-code `DaggerCloud` path, not less — this is reflected in tasks.md giving it
equal (in practice, larger, since it's genuinely more code) task weight, not a stub.

## R8 (Gap 3): concurrency model matches `paws-provision`'s existing `JoinSet` pattern

**Decision**: FR-002a's multi-provider docs-publish concurrency (Clarifications, Session
2026-08-23) and any future multi-target orchestration in this spec reuse
`paws-provision::provision_with_timing`'s exact shape: `tokio::task::JoinSet`, one task per
target, `join_next_with_id` draining every outcome (including panics, attributed to the right
target) into an aggregated `HashMap`/`Vec` — no early return on first failure.

**Rationale**: this is a directly reusable, already-tested pattern in the same workspace
(`crates/paws-provision/src/lib.rs:94-140`) implementing exactly the constitution's "No swallowed
concurrent failures" constraint this spec's Clarifications explicitly invoke — not a new concurrency
primitive to design from scratch.

## Summary of new surface

| Crate | Change |
|---|---|
| `paws-rust` | One-line clippy flag fix (R1) |
| `paws-audit` | New `AUDIT_SCANNER_REGISTRY` entry for `cargo-audit` (R2/R3); new `parse_cargo_audit_findings` |
| `paws-release` | `GitHubReleaseClient` gains `get_pages_config`/`create_blob`/`publish_tree` (R4) |
| `paws-docs` | Gains provider dispatch (`github-pages` implemented; `cloudflare-pages`/`s3` stubbed per FR-004a) |
| `paws-dagger` | Gains `CacheBackend` detection/selection (R6/R7) and wraps existing `core`/`core_streaming` call sites for the Actions-cache provider |
| `paws-cli-core` | `DocsArgs` gains `--provider`/`--repository`/`--branch`; `run_docs` gains multi-provider concurrent dispatch (R8) reusing `paws-provision`'s `JoinSet` shape |
