# Rust Pipeline Contract Matrix

| Surface                                    | Contract                                                       | Status      | Evidence                                                        |
| ------------------------------------------ | -------------------------------------------------------------- | ----------- | --------------------------------------------------------------- |
| `.github/workflows/rust-build-n-test.yaml` | Build/test defaults with optional `publish` and `publish_docs` | Implemented | workflow inputs + `rust-pipeline` call wiring                   |
| `.github/workflows/rust-build-n-test.yaml` | `CARGO_REGISTRY_TOKEN` required only when publish is enabled   | Implemented | `--publish` + `--token` to `rust-pipeline`; fail-fast in Dagger |
| `.github/workflows/rust-docs-publish.yaml` | Shared docs-to-GitHub-Pages reusable path                      | Implemented | new reusable workflow with Pages deploy                         |
| `packages/dagger-module/src/index.ts`      | Public `@func()` orchestration for Rust modes                  | Implemented | `rustPipeline` and `rustDocsBuild`                              |
| `packages/dagger-pipelines/src/*.test.ts`  | Governance coverage for workflow contracts and call names      | Implemented | rust/security/release/workflow-governance test additions        |
