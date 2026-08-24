# Contract: `paws ci --toolchain rust --coverage`

## 1) CLI flag contract (`paws-cli-core::CiArgs`)

| Flag | Type | Gate |
|---|---|---|
| `--coverage` | bool, default `false` | valid only with `--toolchain rust`; any other `--toolchain` value combined with `--coverage` fails with a clear error (research.md R4), before any pipeline runs |

Omitting `--coverage` (the default) MUST produce byte-identical `dagger_pipeline_args` output to
today, for both the wasm and non-wasm paths.

## 2) `paws-rust::dagger_pipeline_args` contract

- Signature extended additively (data-model.md) — `coverage: bool` and `builder_dir: Option<&str>`
  new trailing parameters. Every existing test/call site passing `false`/`None` sees unchanged
  output.
- `coverage == true` on a non-wasm project: opening chain uses `builders/rust`'s `docker-build`
  (research.md R2) instead of a plain `container from`; `cargo test --verbose` is unchanged; one
  new step, `cargo llvm-cov --workspace --summary-only`, is appended after it (research.md R1).
- `coverage == true` on a wasm project (`is_wasm_project` detects it): no-op — the existing wasm
  pipeline runs exactly as it does today, no coverage step, no error (research.md R5).
- Fail-fast semantics unchanged: `paws_dagger::core` aborts the whole pipeline on the first
  non-zero exit, same as today — a failing `cargo test --verbose` still fails the pipeline before
  `cargo llvm-cov` ever runs, whether or not `--coverage` is set.

## 3) `builders/rust/Dockerfile` contract

- Base image: `rust:1-bookworm` (same tag the non-coverage pipeline pulls directly).
- Adds: `llvm-tools-preview` rustup component, `cargo-llvm-cov` binary.
- OCI labels: same `BUILDER_VERSION`/`BUILDER_REVISION`/`BUILDER_CREATED` + `org.opencontainers.image.*`
  shape every other `builders/*/Dockerfile` already carries (`builders/java/Dockerfile` is the
  direct template) — `io.paws.targets="rust"`.
- Never pulled from a registry by `paws ci` itself — always built fresh through Dagger's
  `docker-build` at pipeline-run time (research.md R2), same as `tauri-linux`/`tauri-android`/
  `java`. `compose.yml`'s `rust` service (research.md R3) exists for CI build-verification and
  registry-cache population only, not for `paws ci` to pull from.

## 4) Output contract

- `--coverage` unset: stdout is exactly today's `cargo fmt`/`clippy`/`build`/`test` output, nothing
  added.
- `--coverage` set (non-wasm): stdout is today's output, plus `cargo llvm-cov`'s summary table
  appended at the end — no separate machine-readable file is produced or exported (Out of Scope).
- `--coverage` set (wasm): stdout is exactly today's wasm-pipeline output, unchanged (research.md
  R5) — `--coverage` silently makes no difference on this path.
