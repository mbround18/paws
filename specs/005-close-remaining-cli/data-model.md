# Data Model: Close Remaining Gaps Found Migrating `valheim-docker`

Four independent gaps; this document covers the new/changed shapes each introduces.

## Gap 1: `paws docs` publish

### `PublishTarget` (new enum, `paws-docs`)

```rust
pub enum PublishTarget {
    GitHubPages,
    CloudflarePages, // FR-004a: recognized, fails fast, not implemented
    S3,               // FR-004a: recognized, fails fast, not implemented
}
```

Parsed from `--provider`'s comma-delimited values (`crates/paws-cli-core`'s `DocsArgs.provider:
Vec<String>`, `value_delimiter = ','`, matching `--registries`/`--targets`'s existing convention) —
each value independently mapped to a `PublishTarget` or rejected with a normal clap-shaped
"invalid value" error (Edge Cases) before any work starts.

### `GitHubPagesMechanism` (new enum, internal to the `github-pages` provider)

```rust
enum GitHubPagesMechanism {
    GitTrees,           // build_type == "legacy", or Pages not configured yet (fallback)
    PagesDeployment,     // build_type == "workflow" — requires Actions-runtime env vars (R5)
}
```

Resolved once per publish attempt via `GitHubReleaseClient::get_pages_config` (research.md R4) —
not user-configurable, per FR-003.

### `PublishOutcome` (new struct, `paws-cli-core`, mirrors `paws-provision::ProvisionOutcome`)

```rust
struct PublishOutcome {
    target: PublishTarget,
    result: anyhow::Result<()>,
    elapsed: std::time::Duration,
}
```

One per requested `--provider` value, collected via the `JoinSet` pattern (research.md R8) — never
short-circuited on the first failure (FR-002a).

### `GitHubReleaseClient` additions (`paws-release`)

```rust
pub struct PagesConfig {
    pub build_type: String, // "legacy" | "workflow"
}

impl GitHubReleaseClient {
    pub async fn get_pages_config(&self) -> Result<Option<PagesConfig>>; // None = 404, not configured
    pub async fn create_blob(&self, content: &[u8]) -> Result<String>;   // returns blob sha
    pub async fn publish_tree(
        &self,
        branch: &str,
        files: &[(String, String)], // (repo-relative path, blob sha)
        message: &str,
    ) -> Result<()>;
}
```

## Gap 2: clippy gate

No new data shapes — `paws-rust::dagger_pipeline_args`'s non-wasm branch gains one more
`push_exec` argument set (`-- -D warnings`), no signature change (research.md R1).

## Gap 3: Dagger build cache

### `CacheBackend` (new trait + enum, `paws-dagger`)

```rust
pub enum CacheBackend {
    DaggerCloud,        // DAGGER_CLOUD_TOKEN present — near-zero-code, R6
    GitHubActionsCache,   // ACTIONS_CACHE_URL/ACTIONS_RESULTS_URL present, no DAGGER_CLOUD_TOKEN — R7
    None,                 // neither signature present — today's behavior, unchanged
}

impl CacheBackend {
    /// FR-005: fixed precedence — DaggerCloud wins when both signatures match.
    pub fn detect() -> Self;
}
```

`CacheBackend::GitHubActionsCache` additionally wraps the existing `paws_dagger::core`/
`core_streaming` call sites with a restore-before/save-after pair against the Actions Cache REST
API (research.md R7) — the exact restore/save boundary and cache-key scheme (likely keyed on the
Dagger engine version + a rolling/always-save policy, resolved in tasks.md) don't change any
existing function's public signature; they wrap the same two entry points every subcommand already
calls.

### Log line contract (FR-006)

Every `paws docker`/`paws ci` invocation logs exactly one line naming the selected backend (or
`none`) at the point `CacheBackend::detect()` runs — e.g. `cache: using dagger-cloud` /
`cache: using github-actions` / `cache: no backend detected (full rebuild)` — independently
verifiable without inferring from build speed.

## Gap 4: Rust dependency scanner

### `AUDIT_SCANNER_REGISTRY` addition (`paws-audit`)

```rust
(
    ScannerName::CargoAudit,              // new ScannerName variant
    &[LanguageFamily::Rust],              // applies_to
    "cargo audit --json",                  // step_name
    "rust:1-bookworm",                     // image — cargo-audit installed at scan time, research.md R3
),
```

`ScannerName` gains one variant (`CargoAudit`, `as_str() -> "cargo-audit"`); `ScannerConfig` for
this entry uses `ScannerFamily::Language(LanguageFamily::Rust)` (the existing, currently-unused
enum variant, research.md R2) instead of `ScannerFamily::CrossLanguage`.

### `parse_cargo_audit_findings` (new function, mirrors `parse_semgrep_findings`)

```rust
fn parse_cargo_audit_findings(raw_json: &str) -> (usize, Vec<TopFinding>);
```

Maps each `vulnerabilities.list[]` entry (research.md R3's schema) to one `TopFinding`, same
shape every other scanner's parser already produces — no new field on `AuditScannerResult`/
`TopFinding` themselves.
