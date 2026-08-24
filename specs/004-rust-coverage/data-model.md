# Data Model: `paws ci --toolchain rust --coverage`

This feature has no persistent entities or config schema — it's a pipeline-argument-construction
change. This document covers the one shape that changes: `paws-rust::dagger_pipeline_args`'s
signature and the pipeline chain it builds.

## `dagger_pipeline_args` (extended)

```rust
pub fn dagger_pipeline_args(
    source_dir: &str,
    is_wasm: bool,
    coverage: bool,       // NEW
    builder_dir: Option<&str>, // NEW — Some(dir) when coverage is true (R2); None otherwise
) -> Vec<String>
```

Existing call sites (today's only caller, `run_ci` with coverage always `false`/`None`) see
byte-identical output when `coverage` is `false` — mirrors `003-release-parity-docker`'s own
`generate_tags`/`generate_tag_matrix` backward-compatibility approach (this spec's own FR/SC
equivalent: default behavior stays byte-identical when the new flag is omitted).

### Opening chain (conditional on `coverage`)

- `coverage == false` (today's only path): `container from --address=rust:1-bookworm`, unchanged.
- `coverage == true`: `host directory --path=<builder_dir> docker-build`, where `builder_dir` is
  the temp directory `write_builder_dockerfile()` (new, mirrors `paws-tauri`'s/`paws-java`'s own
  same-named function) materializes `builders/rust/Dockerfile` into.

### Step sequence (after the opening chain, both cases)

Unchanged from today for `is_wasm == true` (coverage is a no-op there, research.md R5):
`rustup target add` → `rustup component add rustfmt clippy` → `cargo fmt -- --check` →
`cargo clippy --target ... -- -D warnings` → `cargo build --target ... --verbose`.

For `is_wasm == false`:
- `coverage == false` (today): `rustup component add rustfmt clippy` → `cargo fmt -- --check` →
  `cargo clippy` → `cargo build --verbose` → `cargo test --verbose`.
- `coverage == true`: same five steps, **plus** one more appended at the end:
  `cargo llvm-cov --workspace --summary-only` (research.md R1). `cargo test --verbose` itself is
  unchanged (Clarifications, Session 2026-08-23) — this is a pure append, not a step replacement.

### `builders/rust/Dockerfile` (new)

No runtime-configurable fields — a static image definition, `ARG`s only for the shared
`BUILDER_VERSION`/`BUILDER_REVISION`/`BUILDER_CREATED` OCI-label triad every `builders/*`
Dockerfile already uses (`builders/java/Dockerfile` is the direct template). Base:
`rust:1-bookworm` (same tag `paws-rust`'s default, non-coverage pipeline already pulls — this
builder image tracks it, doesn't fork from it), plus:
- `rustup component add llvm-tools-preview`
- `cargo install cargo-llvm-cov`

## `CiArgs` (extended)

```rust
pub struct CiArgs {
    // ...existing fields unchanged...
    pub coverage: bool, // NEW — default false, `#[arg(long)]`/`#[serde(default)]`
}
```

`run_ci` validates `coverage && toolchain.as_deref() != Some("rust")` as an error case (research.md
R4) before dispatching to `paws_rust::dagger_pipeline_args`.
