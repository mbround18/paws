# Contract: `cargo-audit` scanner (`paws audit`)

## 1) Catalog contract

New `AUDIT_SCANNER_REGISTRY` row: `ScannerName::CargoAudit`, `applies_to: &[LanguageFamily::Rust]`,
`ScannerFamily::Language(LanguageFamily::Rust)`. Gating goes through the existing
`select_audit_scanners` function unmodified (FR-010) — a non-Rust project's `should_run` resolves
`false` via the same `detected_families.contains(&LanguageFamily::Rust)` check every other scanner
already uses.

## 2) Output contract (FR-009)

`cargo audit --json`'s `vulnerabilities.list[]` parses into `AuditScannerResult`/`TopFinding` via
a new `parse_cargo_audit_findings`, mirroring `parse_semgrep_findings`'s exact mapping shape — no
new field on either struct.

## 3) Default-behavior contract (Assumptions)

Reports findings without failing the build by default, matching Semgrep/Gitleaks's *confirmed*
(not assumed) current behavior — verified directly in the Validation Plan before this scanner
ships, not inferred.

## 4) Rollout contract

Additive to the existing catalog — every current `paws audit` consumer with a Rust project sees
this scanner run automatically on their next `paws audit`, no flag needed (matching how
Semgrep/Gitleaks already apply once their language signal is detected).
