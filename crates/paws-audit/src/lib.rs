//! Rust port of `gh-reusable`'s audit/compliance aggregation logic.
//!
//! Parity source (read directly): `packages/dagger-module/src/audit-types.ts` (shapes) and
//! `packages/dagger-module/src/audit-logic.ts` (detection, scanner selection, aggregation,
//! finding normalization). This crate only ports the pure aggregation/detection logic —
//! actually running `semgrep`/`gitleaks` is still `paws-dagger`'s job (see `paws-cli`'s
//! `Audit` handler); this crate turns their raw output into the same summary shape
//! downstream tooling already consumes (spec.md User Story 4, FR-006).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LanguageFamily {
    Rust,
    Node,
    Python,
    Go,
    Docker,
    Generic,
}

impl LanguageFamily {
    pub fn as_str(&self) -> &'static str {
        match self {
            LanguageFamily::Rust => "rust",
            LanguageFamily::Node => "node",
            LanguageFamily::Python => "python",
            LanguageFamily::Go => "go",
            LanguageFamily::Docker => "docker",
            LanguageFamily::Generic => "generic",
        }
    }
}

const LANGUAGE_FAMILIES: &[LanguageFamily] = &[
    LanguageFamily::Rust,
    LanguageFamily::Node,
    LanguageFamily::Python,
    LanguageFamily::Go,
    LanguageFamily::Docker,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DetectionConfidence {
    None,
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedFamily {
    pub family: LanguageFamily,
    pub confidence: DetectionConfidence,
    pub signals: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectionResult {
    pub families: Vec<DetectedFamily>,
    pub fallback_mode: bool,
    pub high_confidence_families: Vec<LanguageFamily>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScannerStatus {
    Pass,
    Findings,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopFinding {
    pub rule: String,
    pub severity: Severity,
    pub path: String,
    pub line: Option<u32>,
    pub message: String,
    pub scanner: String,
}

/// `LanguageFamily | "cross-language"` in the TS source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScannerFamily {
    Language(LanguageFamily),
    CrossLanguage,
}

impl ScannerFamily {
    fn as_str(&self) -> &'static str {
        match self {
            ScannerFamily::Language(family) => family.as_str(),
            ScannerFamily::CrossLanguage => "cross-language",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditScannerResult {
    pub name: String,
    pub family: ScannerFamily,
    pub status: ScannerStatus,
    pub findings_count: usize,
    pub duration_ms: u64,
    pub failure_reason: Option<String>,
    pub top_findings: Vec<TopFinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditOverallStatus {
    Pass,
    Findings,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditSummary {
    pub overall_status: AuditOverallStatus,
    pub detected_families: Vec<String>,
    pub detection_confidence: DetectionConfidence,
    pub fallback_mode: bool,
    pub scanners: Vec<AuditScannerResult>,
    pub total_findings: usize,
    pub top_findings: Vec<TopFinding>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScannerName {
    Semgrep,
    Gitleaks,
}

impl ScannerName {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScannerName::Semgrep => "semgrep",
            ScannerName::Gitleaks => "gitleaks",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannerConfig {
    pub name: ScannerName,
    pub family: ScannerFamily,
    pub applies_to: Vec<LanguageFamily>,
    pub should_run: bool,
    pub step_name: String,
    pub image: String,
}

/// Repository file-presence signals keyed by filename, e.g. `{"Cargo.toml": true}`.
pub type RepositorySignals = HashMap<String, bool>;

fn signal(signals: &RepositorySignals, name: &str) -> bool {
    signals.get(name).copied().unwrap_or(false)
}

fn detect_with_signals(
    family: LanguageFamily,
    signals: &RepositorySignals,
    entries: &[(&str, DetectionConfidence)],
    high_confidence_combos: &[&[&str]],
) -> DetectedFamily {
    let present: Vec<(&str, DetectionConfidence)> = entries
        .iter()
        .copied()
        .filter(|(name, _)| signal(signals, name))
        .collect();

    if present.is_empty() {
        return DetectedFamily {
            family,
            confidence: DetectionConfidence::None,
            signals: vec![],
        };
    }

    let has_high_signal = high_confidence_combos
        .iter()
        .any(|combo| combo.iter().all(|name| signal(signals, name)));

    let confidence = if has_high_signal {
        DetectionConfidence::High
    } else if present.iter().any(|(_, c)| *c == DetectionConfidence::Medium) {
        DetectionConfidence::Medium
    } else {
        DetectionConfidence::Low
    };

    DetectedFamily {
        family,
        confidence,
        signals: present.into_iter().map(|(name, _)| name.to_string()).collect(),
    }
}

fn detect_family(signals: &RepositorySignals, family: LanguageFamily) -> DetectedFamily {
    use DetectionConfidence::{Low, Medium};
    match family {
        LanguageFamily::Rust => detect_with_signals(
            family,
            signals,
            &[("Cargo.toml", Medium), ("Cargo.lock", Low)],
            &[&["Cargo.toml", "Cargo.lock"]],
        ),
        LanguageFamily::Node => detect_with_signals(
            family,
            signals,
            &[
                ("package.json", Medium),
                ("pnpm-lock.yaml", Low),
                ("yarn.lock", Low),
                ("package-lock.json", Low),
            ],
            &[
                &["package.json", "pnpm-lock.yaml"],
                &["package.json", "yarn.lock"],
                &["package.json", "package-lock.json"],
            ],
        ),
        LanguageFamily::Python => detect_with_signals(
            family,
            signals,
            &[
                ("pyproject.toml", Medium),
                ("uv.lock", Low),
                ("poetry.lock", Low),
                ("requirements.txt", Medium),
                ("setup.py", Low),
            ],
            &[&["pyproject.toml", "uv.lock"], &["pyproject.toml", "poetry.lock"]],
        ),
        LanguageFamily::Go => detect_with_signals(
            family,
            signals,
            &[("go.mod", Medium), ("go.sum", Low)],
            &[&["go.mod", "go.sum"]],
        ),
        LanguageFamily::Docker => detect_with_signals(
            family,
            signals,
            &[
                ("Dockerfile", DetectionConfidence::High),
                ("docker-compose.yml", Medium),
                ("docker-compose.yaml", Medium),
                ("compose.yml", Medium),
                ("compose.yaml", Medium),
            ],
            &[&["Dockerfile"]],
        ),
        LanguageFamily::Generic => DetectedFamily {
            family,
            confidence: DetectionConfidence::High,
            signals: vec!["(baseline)".to_string()],
        },
    }
}

/// Ported from `detectLanguageFamilies`.
pub fn detect_language_families(signals: &RepositorySignals) -> DetectionResult {
    let mut families: Vec<DetectedFamily> = LANGUAGE_FAMILIES
        .iter()
        .map(|family| detect_family(signals, *family))
        .filter(|detected| detected.confidence != DetectionConfidence::None)
        .collect();

    families.push(detect_family(signals, LanguageFamily::Generic));

    let non_generic: Vec<&DetectedFamily> = families
        .iter()
        .filter(|f| f.family != LanguageFamily::Generic)
        .collect();
    let fallback_mode = !non_generic
        .iter()
        .any(|f| f.confidence >= DetectionConfidence::Medium);
    let high_confidence_families = non_generic
        .iter()
        .filter(|f| f.confidence == DetectionConfidence::High)
        .map(|f| f.family)
        .collect();

    DetectionResult {
        families,
        fallback_mode,
        high_confidence_families,
    }
}

const AUDIT_SCANNER_REGISTRY: &[(ScannerName, &[LanguageFamily], &str, &str)] = &[
    (
        ScannerName::Semgrep,
        &[
            LanguageFamily::Rust,
            LanguageFamily::Node,
            LanguageFamily::Python,
            LanguageFamily::Go,
            LanguageFamily::Docker,
        ],
        "semgrep scan",
        "returntocorp/semgrep:1.81.0",
    ),
    (
        ScannerName::Gitleaks,
        &[
            LanguageFamily::Rust,
            LanguageFamily::Node,
            LanguageFamily::Python,
            LanguageFamily::Go,
            LanguageFamily::Docker,
        ],
        "gitleaks detect",
        "zricethezav/gitleaks:v8.24.2",
    ),
];

/// Ported from `selectAuditScanners`.
pub fn select_audit_scanners(detection: &DetectionResult, include_gitleaks: bool) -> Vec<ScannerConfig> {
    let detected_families: std::collections::HashSet<LanguageFamily> =
        detection.families.iter().map(|f| f.family).collect();

    AUDIT_SCANNER_REGISTRY
        .iter()
        .map(|(name, applies_to, step_name, image)| {
            let should_run = (*name != ScannerName::Gitleaks || include_gitleaks)
                && applies_to.iter().any(|family| detected_families.contains(family));
            ScannerConfig {
                name: *name,
                family: ScannerFamily::CrossLanguage,
                applies_to: applies_to.to_vec(),
                should_run,
                step_name: step_name.to_string(),
                image: image.to_string(),
            }
        })
        .collect()
}

/// Ported from `createSkippedScannerResult`.
pub fn create_skipped_scanner_result(scanner: &ScannerConfig) -> AuditScannerResult {
    AuditScannerResult {
        name: scanner.name.as_str().to_string(),
        family: scanner.family,
        status: ScannerStatus::Skipped,
        findings_count: 0,
        duration_ms: 0,
        failure_reason: None,
        top_findings: vec![],
    }
}

/// Ported from `createFailedScannerResult`.
pub fn create_failed_scanner_result(
    scanner: &ScannerConfig,
    duration_ms: u64,
    failure_reason: String,
) -> AuditScannerResult {
    AuditScannerResult {
        name: scanner.name.as_str().to_string(),
        family: scanner.family,
        status: ScannerStatus::Failed,
        findings_count: 0,
        duration_ms,
        failure_reason: Some(failure_reason),
        top_findings: vec![],
    }
}

/// Ported from `normalizeScannerStatus` (never returns `Skipped`).
pub fn normalize_scanner_status(exit_code: Option<i32>, findings_count: usize) -> ScannerStatus {
    if exit_code == Some(0) {
        if findings_count > 0 {
            ScannerStatus::Findings
        } else {
            ScannerStatus::Pass
        }
    } else if findings_count > 0 {
        ScannerStatus::Findings
    } else {
        ScannerStatus::Failed
    }
}

fn severity_rank(severity: Severity) -> u8 {
    severity as u8
}

fn compare_top_findings(left: &TopFinding, right: &TopFinding) -> std::cmp::Ordering {
    severity_rank(left.severity)
        .cmp(&severity_rank(right.severity))
        .then_with(|| left.scanner.cmp(&right.scanner))
        .then_with(|| left.path.cmp(&right.path))
        .then_with(|| left.line.unwrap_or(0).cmp(&right.line.unwrap_or(0)))
}

fn scanner_status_rank(status: ScannerStatus) -> u8 {
    match status {
        ScannerStatus::Failed => 0,
        ScannerStatus::Findings => 1,
        ScannerStatus::Pass => 2,
        ScannerStatus::Skipped => 3,
    }
}

fn sort_scanner_results(scanners: &[AuditScannerResult]) -> Vec<AuditScannerResult> {
    let mut sorted = scanners.to_vec();
    sorted.sort_by(|left, right| {
        scanner_status_rank(left.status)
            .cmp(&scanner_status_rank(right.status))
            .then_with(|| right.findings_count.cmp(&left.findings_count))
            .then_with(|| left.name.cmp(&right.name))
    });
    sorted
}

fn derive_detection_confidence(detection: &DetectionResult) -> DetectionConfidence {
    let non_generic: Vec<&DetectedFamily> = detection
        .families
        .iter()
        .filter(|f| f.family != LanguageFamily::Generic)
        .collect();
    if non_generic.is_empty() {
        return DetectionConfidence::None;
    }
    non_generic
        .iter()
        .map(|f| f.confidence)
        .min()
        .unwrap_or(DetectionConfidence::None)
}

fn derive_overall_status(scanners: &[AuditScannerResult]) -> AuditOverallStatus {
    if scanners.is_empty() {
        return AuditOverallStatus::Failed;
    }
    let failed = scanners.iter().filter(|s| s.status == ScannerStatus::Failed).count();
    let has_findings = scanners.iter().any(|s| s.status == ScannerStatus::Findings);
    if failed == scanners.len() {
        AuditOverallStatus::Failed
    } else if failed > 0 {
        AuditOverallStatus::Degraded
    } else if has_findings {
        AuditOverallStatus::Findings
    } else {
        AuditOverallStatus::Pass
    }
}

/// Ported from `aggregateAuditResults`: confidence ranking plus failed/skipped
/// scanner handling (task 36's port target).
pub fn aggregate_audit_results(
    scanner_results: &[AuditScannerResult],
    detection: &DetectionResult,
) -> AuditSummary {
    let scanners = sort_scanner_results(scanner_results);
    let runnable: Vec<&AuditScannerResult> = scanners.iter().filter(|s| s.status != ScannerStatus::Skipped).collect();

    let total_findings: usize = runnable
        .iter()
        .filter(|s| matches!(s.status, ScannerStatus::Findings | ScannerStatus::Pass))
        .map(|s| s.findings_count)
        .sum();

    let mut top_findings: Vec<TopFinding> = runnable.iter().flat_map(|s| s.top_findings.clone()).collect();
    top_findings.sort_by(compare_top_findings);
    top_findings.truncate(10);

    let overall_status = derive_overall_status(&runnable.into_iter().cloned().collect::<Vec<_>>());

    AuditSummary {
        overall_status,
        detected_families: detection.families.iter().map(|f| f.family.as_str().to_string()).collect(),
        detection_confidence: derive_detection_confidence(detection),
        fallback_mode: detection.fallback_mode,
        scanners,
        total_findings,
        top_findings,
    }
}

/// Ported from `buildScanFindings`.
pub fn build_scan_findings(summary: &AuditSummary) -> HashMap<String, i64> {
    let by_name = |name: &str| -> i64 {
        summary
            .scanners
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.findings_count as i64)
            .unwrap_or(0)
    };

    let mut result = HashMap::new();
    result.insert("semgrep".to_string(), by_name("semgrep"));
    result.insert("gitleaks".to_string(), by_name("gitleaks"));
    result.insert("total".to_string(), summary.total_findings as i64);
    result.insert(
        "detectedFamilyCount".to_string(),
        summary.detected_families.iter().filter(|f| f.as_str() != "generic").count() as i64,
    );
    result.insert("fallbackMode".to_string(), i64::from(summary.fallback_mode));
    result.insert(
        "scannerFailureCount".to_string(),
        summary.scanners.iter().filter(|s| s.status == ScannerStatus::Failed).count() as i64,
    );
    result
}

fn status_icon(status: ScannerStatus) -> &'static str {
    match status {
        ScannerStatus::Pass => "OK",
        ScannerStatus::Findings => "WARN",
        ScannerStatus::Failed => "FAIL",
        ScannerStatus::Skipped => "SKIP",
    }
}

/// Ported from `renderAuditIntelligenceSection` (plain-text status markers instead of
/// emoji, everything else matches the original's structure/ordering).
pub fn render_audit_intelligence_section(summary: &AuditSummary) -> String {
    let families = summary
        .detected_families
        .iter()
        .map(|f| if f == "generic" { "generic (baseline)".to_string() } else { f.clone() })
        .collect::<Vec<_>>()
        .join(", ");

    let scanner_rows = summary
        .scanners
        .iter()
        .map(|s| {
            format!(
                "| {} | {} `{:?}` | {} | {} | {} |",
                s.name,
                status_icon(s.status),
                s.status,
                s.findings_count,
                s.duration_ms,
                s.family.as_str()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let failed_count = summary.scanners.iter().filter(|s| s.status == ScannerStatus::Failed).count();

    let mut lines = vec![
        "### Audit Intelligence".to_string(),
        String::new(),
        "| Field | Value |".to_string(),
        "| --- | --- |".to_string(),
        format!("| Overall status | `{:?}` |", summary.overall_status),
        format!("| Detection confidence | `{:?}` |", summary.detection_confidence),
        format!(
            "| Detected families | {} |",
            if families.is_empty() { "_(none)_".to_string() } else { format!("`{families}`") }
        ),
        format!("| Fallback mode | `{}` |", summary.fallback_mode),
        format!("| Scanner failures | `{failed_count}` |"),
        String::new(),
        "#### Scanners".to_string(),
        String::new(),
        "| Scanner | Status | Findings | Duration (ms) | Family |".to_string(),
        "| --- | --- | --- | --- | --- |".to_string(),
        if scanner_rows.is_empty() {
            "| _(none)_ | SKIP `skipped` | `0` | `0` | _(none)_ |".to_string()
        } else {
            scanner_rows
        },
        String::new(),
        "#### Top findings".to_string(),
        String::new(),
    ];

    if summary.top_findings.is_empty() {
        lines.push("- (none)".to_string());
    } else {
        for (index, finding) in summary.top_findings.iter().enumerate() {
            let line = finding.line.map(|l| format!(":{l}")).unwrap_or_default();
            lines.push(format!(
                "{}. **{:?}** `{}` · `{}` · `{}{}` — {}",
                index + 1,
                finding.severity,
                finding.scanner,
                finding.rule,
                finding.path,
                line,
                finding.message
            ));
        }
    }

    if summary.fallback_mode {
        lines.push(String::new());
        lines.push(
            "> Fallback mode was used because repository language detection was weak or ambiguous."
                .to_string(),
        );
    }

    lines.into_iter().filter(|l| !l.is_empty() || true).collect::<Vec<_>>().join("\n")
}

fn normalize_severity(value: Option<&str>) -> Severity {
    match value.unwrap_or("").to_lowercase().as_str() {
        "critical" | "error" => Severity::Critical,
        "high" => Severity::High,
        "medium" | "moderate" => Severity::Medium,
        "low" => Severity::Low,
        _ => Severity::Info,
    }
}

fn parse_semgrep_findings(raw_json: &str) -> (usize, Vec<TopFinding>) {
    let parsed: serde_json::Value = serde_json::from_str(raw_json).unwrap_or(serde_json::Value::Null);
    let results = parsed.get("results").and_then(|v| v.as_array()).cloned().unwrap_or_default();

    let mut top_findings: Vec<TopFinding> = results
        .iter()
        .filter_map(|result| {
            let extra = result.get("extra");
            let start = result.get("start");
            let path = result
                .get("path")
                .and_then(|v| v.as_str())
                .or_else(|| extra.and_then(|e| e.get("path")).and_then(|v| v.as_str()))?
                .to_string();

            Some(TopFinding {
                rule: result
                    .get("check_id")
                    .and_then(|v| v.as_str())
                    .or_else(|| extra.and_then(|e| e.get("check_id")).and_then(|v| v.as_str()))
                    .unwrap_or("semgrep-rule")
                    .to_string(),
                severity: normalize_severity(
                    extra
                        .and_then(|e| e.get("severity"))
                        .and_then(|v| v.as_str())
                        .or_else(|| result.get("severity").and_then(|v| v.as_str())),
                ),
                path,
                line: start
                    .and_then(|s| s.get("line"))
                    .and_then(|v| v.as_u64())
                    .or_else(|| result.get("line").and_then(|v| v.as_u64()))
                    .map(|v| v as u32),
                message: extra
                    .and_then(|e| e.get("message"))
                    .and_then(|v| v.as_str())
                    .or_else(|| result.get("message").and_then(|v| v.as_str()))
                    .unwrap_or("Semgrep finding")
                    .to_string(),
                scanner: "semgrep".to_string(),
            })
        })
        .collect();
    top_findings.sort_by(compare_top_findings);
    top_findings.truncate(10);

    (results.len(), top_findings)
}

fn parse_gitleaks_findings(raw_json: &str) -> (usize, Vec<TopFinding>) {
    let parsed: serde_json::Value = serde_json::from_str(raw_json).unwrap_or(serde_json::Value::Null);
    let results = parsed.as_array().cloned().unwrap_or_default();

    let str_field = |v: &serde_json::Value, keys: &[&str]| -> Option<String> {
        keys.iter().find_map(|k| v.get(k).and_then(|x| x.as_str()).map(String::from))
    };
    let num_field = |v: &serde_json::Value, keys: &[&str]| -> Option<u32> {
        keys.iter().find_map(|k| v.get(k).and_then(|x| x.as_u64()).map(|n| n as u32))
    };

    let mut top_findings: Vec<TopFinding> = results
        .iter()
        .filter_map(|result| {
            let path = str_field(result, &["File", "file", "Path"])?;
            Some(TopFinding {
                rule: str_field(result, &["RuleID", "ruleID", "rule_id"]).unwrap_or_else(|| "gitleaks-rule".to_string()),
                severity: normalize_severity(
                    str_field(result, &["Severity", "severity"]).as_deref().or(Some("high")),
                ),
                path,
                line: num_field(result, &["StartLine", "startLine", "Line", "line"]),
                message: str_field(result, &["Description", "description", "Message", "message"])
                    .unwrap_or_else(|| "Gitleaks finding".to_string()),
                scanner: "gitleaks".to_string(),
            })
        })
        .collect();
    top_findings.sort_by(compare_top_findings);
    top_findings.truncate(10);

    (results.len(), top_findings)
}

/// Ported from `parseScannerFindings`.
pub fn parse_scanner_findings(scanner: ScannerName, raw_json: &str) -> (usize, Vec<TopFinding>) {
    match scanner {
        ScannerName::Semgrep => parse_semgrep_findings(raw_json),
        ScannerName::Gitleaks => parse_gitleaks_findings(raw_json),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signals(present: &[&str]) -> RepositorySignals {
        present.iter().map(|s| (s.to_string(), true)).collect()
    }

    #[test]
    fn no_findings_fixture_passes_with_empty_summary() {
        let detection = detect_language_families(&signals(&["Cargo.toml", "Cargo.lock"]));
        let scanners = select_audit_scanners(&detection, true);

        let results: Vec<AuditScannerResult> = scanners
            .iter()
            .map(|scanner| {
                if scanner.should_run {
                    AuditScannerResult {
                        name: scanner.name.as_str().to_string(),
                        family: scanner.family,
                        status: ScannerStatus::Pass,
                        findings_count: 0,
                        duration_ms: 42,
                        failure_reason: None,
                        top_findings: vec![],
                    }
                } else {
                    create_skipped_scanner_result(scanner)
                }
            })
            .collect();

        let summary = aggregate_audit_results(&results, &detection);

        assert_eq!(summary.overall_status, AuditOverallStatus::Pass);
        assert_eq!(summary.total_findings, 0);
        assert!(summary.top_findings.is_empty());
        assert!(!summary.fallback_mode);
        assert_eq!(summary.detection_confidence, DetectionConfidence::High);
    }

    #[test]
    fn single_finding_appears_in_summary_with_matching_fields() {
        let detection = detect_language_families(&signals(&["Cargo.toml", "Cargo.lock"]));

        let raw_semgrep = r#"{"results":[{"check_id":"rust.lang.security.foo","path":"src/lib.rs","start":{"line":10},"extra":{"severity":"ERROR","message":"do not do this"}}]}"#;
        let (findings_count, top_findings) = parse_scanner_findings(ScannerName::Semgrep, raw_semgrep);

        let scanner_result = AuditScannerResult {
            name: "semgrep".to_string(),
            family: ScannerFamily::CrossLanguage,
            status: normalize_scanner_status(Some(1), findings_count),
            findings_count,
            duration_ms: 100,
            failure_reason: None,
            top_findings,
        };

        let summary = aggregate_audit_results(&[scanner_result], &detection);

        assert_eq!(summary.overall_status, AuditOverallStatus::Findings);
        assert_eq!(summary.total_findings, 1);
        assert_eq!(summary.top_findings.len(), 1);
        let finding = &summary.top_findings[0];
        assert_eq!(finding.rule, "rust.lang.security.foo");
        assert_eq!(finding.path, "src/lib.rs");
        assert_eq!(finding.line, Some(10));
        assert_eq!(finding.severity, Severity::Critical);
        assert_eq!(finding.scanner, "semgrep");
    }

    #[test]
    fn detect_language_families_falls_back_when_no_medium_or_high_signal() {
        let detection = detect_language_families(&signals(&[]));
        assert!(detection.fallback_mode);
        assert_eq!(detection.families.len(), 1);
        assert_eq!(detection.families[0].family, LanguageFamily::Generic);
    }

    #[test]
    fn gitleaks_skipped_when_not_requested() {
        let detection = detect_language_families(&signals(&["Cargo.toml"]));
        let scanners = select_audit_scanners(&detection, false);
        let gitleaks = scanners.iter().find(|s| s.name == ScannerName::Gitleaks).unwrap();
        assert!(!gitleaks.should_run);
    }
}
