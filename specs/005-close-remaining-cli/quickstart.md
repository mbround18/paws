# Quickstart: Validating the Four Gaps

Each gap is independently testable per the spec's own User Stories — validate them separately.

## Gap 2 (P1): clippy `-D warnings` on the non-wasm path

```bash
# Introduce a deliberate, harmless clippy warning in a fixture, then:
cd examples/rust-fixture && ../../target/debug/paws ci --toolchain rust
```

Expect: non-zero exit, warning surfaced in output — where before this fix it exited 0. Revert the
warning and re-run: expect a clean pass, unchanged from before this feature (SC-002).

## Gap 3 (P2): `CacheBackend` selection and reuse

```bash
# No backend:
unset DAGGER_CLOUD_TOKEN ACTIONS_CACHE_URL ACTIONS_RESULTS_URL
paws docker --image test/cache-check --version 0.0.1
# Expect: "cache: no backend detected (full rebuild)" log line, behavior unchanged from today.

# DaggerCloud (needs a real Dagger Cloud token):
DAGGER_CLOUD_TOKEN=... paws docker --image test/cache-check --version 0.0.1
# Expect: "cache: using dagger-cloud" log line.
```

For the GitHub Actions cache provider and the real two-separate-environment timing comparison
(SC-004), this needs two genuinely separate CI job runs — not reproducible in a single local
shell session (local filesystem/Docker-layer reuse would confound the measurement). Run as two
sequential jobs in a scratch workflow, comparing wall-clock time on the second run.

## Gap 4 (P2): `cargo-audit` scanner

```bash
# Fixture with a known-vulnerable pinned dependency:
cd <fixture-with-a-RustSec-advisory-in-Cargo.lock> && paws audit
```

Expect: the finding appears in the audit summary alongside (independently of) any Semgrep/Gitleaks
findings, with the same `AuditScannerResult`/`TopFinding` shape. Re-run against a clean fixture:
expect zero findings from this scanner, no effect on overall audit outcome (SC-005).

## Gap 1 (P3): `paws docs --provider github-pages`

```bash
cargo build -p paws-cli
cd <fixture-workspace-with-doc-comments>
../../target/debug/paws docs --provider github-pages --repository <owner>/<scratch-repo>
```

Expect: `target/doc` built once, then published via the auto-selected mechanism (Git Trees API for
a repo with no Pages configured yet, or one using the classic branch-based Pages source). Re-run
identically: expect an idempotent no-op/no-diff (not a duplicate commit).

```bash
# Multi-provider, one implemented one not:
../../target/debug/paws docs --provider github-pages,s3 --repository <owner>/<scratch-repo>
```

Expect: `target/doc` built exactly once; `github-pages` succeeds; `s3` fails with the FR-004a
"not implemented yet" error; both outcomes reported; process exits non-zero.

**Do not run any of these against a real, in-use repository without `--repository` pointed at a
scratch target** — `github-pages`'s publish path writes real commits/deployments.

## Definition of done

- `cargo test --workspace` passes with zero failures, zero regressions in any existing
  `paws-rust`/`paws-docs`/`paws-docker`/`paws-audit` test (spec's Validation Plan).
- Every scenario above is backed by an actual test in the relevant crate (tasks.md enumerates them
  1:1) — this quickstart is a validation guide, not a substitute for the test suite.
