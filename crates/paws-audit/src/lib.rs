//! Rust port of `gh-reusable`'s audit/compliance aggregation logic.
//!
//! Parity source (read directly): `packages/dagger-module/src/audit-types.ts` (shapes) and
//! `packages/dagger-module/src/audit-logic.ts` (detection, scanner selection, aggregation,
//! finding normalization, and — as of the scanner-execution functions below —
//! `runSemgrepScanner`/`runGitleaksScanner` in `packages/dagger-module/src/index.ts`, ported
//! byte-for-byte: same images, same commands, same empty-output fallback). `paws-cli`'s
//! `Audit` handler drives [`scanner_json_pipeline_args`]/[`scanner_exit_code_pipeline_args`]
//! (one call each per [`ScannerConfig`], covering both scanners) through
//! `paws-dagger::core` directly — no `gh-reusable` Dagger Function call anywhere in `paws
//! audit` anymore (it was the last subcommand still depending on `gh-reusable` at all).
//! Real scanner catalog beyond semgrep/gitleaks (the 95+ scanners across every language
//! `audit-mcp` <https://github.com/mbround18/audit-mcp> already catalogs and knows how to
//! run) is a deliberately separate, later expansion — this crate's `ScannerName`/
//! `AUDIT_SCANNER_REGISTRY` are already shaped to add more the same way
//! (`ScannerConfig.image` + a script), but doing all of them at once wasn't this pass's
//! scope. `audit-mcp` itself runs scanners via a direct Docker API client (`bollard`), not
//! Dagger — deliberately not reused as-is here despite the "based on audit-mcp" starting
//! point, since routing all container execution through Dagger (never a direct Docker
//! spawn) is a hard invariant elsewhere in `paws` (see `docs/adr/0001`); what's actually
//! reused is its scanner-catalog *shape* (name/image/command), not its execution engine.

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
    CargoAudit,
}

impl ScannerName {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScannerName::Semgrep => "semgrep",
            ScannerName::Gitleaks => "gitleaks",
            ScannerName::CargoAudit => "cargo-audit",
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
    } else if present
        .iter()
        .any(|(_, c)| *c == DetectionConfidence::Medium)
    {
        DetectionConfidence::Medium
    } else {
        DetectionConfidence::Low
    };

    DetectedFamily {
        family,
        confidence,
        signals: present
            .into_iter()
            .map(|(name, _)| name.to_string())
            .collect(),
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
            &[
                &["pyproject.toml", "uv.lock"],
                &["pyproject.toml", "poetry.lock"],
            ],
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

const AUDIT_SCANNER_REGISTRY: &[(ScannerName, ScannerFamily, &[LanguageFamily], &str, &str)] = &[
    (
        ScannerName::Semgrep,
        ScannerFamily::CrossLanguage,
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
        ScannerFamily::CrossLanguage,
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
    (
        ScannerName::CargoAudit,
        ScannerFamily::Language(LanguageFamily::Rust),
        &[LanguageFamily::Rust],
        "cargo audit --json",
        "rust:1-bookworm",
    ),
];

/// Ported from `selectAuditScanners`.
pub fn select_audit_scanners(
    detection: &DetectionResult,
    include_gitleaks: bool,
) -> Vec<ScannerConfig> {
    let detected_families: std::collections::HashSet<LanguageFamily> =
        detection.families.iter().map(|f| f.family).collect();

    AUDIT_SCANNER_REGISTRY
        .iter()
        .map(|(name, family, applies_to, step_name, image)| {
            let should_run = (*name != ScannerName::Gitleaks || include_gitleaks)
                && applies_to
                    .iter()
                    .any(|family| detected_families.contains(family));
            ScannerConfig {
                name: *name,
                family: *family,
                applies_to: applies_to.to_vec(),
                should_run,
                step_name: step_name.to_string(),
                image: image.to_string(),
            }
        })
        .collect()
}

/// Where a scanner's JSON report ends up inside its own container — needed
/// both to build the run script (below) and to know what path to read back
/// afterward.
fn scanner_output_path(name: ScannerName) -> &'static str {
    match name {
        ScannerName::Semgrep => "/tmp/semgrep.json",
        ScannerName::Gitleaks => "/tmp/gitleaks.json",
        ScannerName::CargoAudit => "/tmp/cargo-audit.json",
    }
}

/// The `sh` script each scanner runs — parity ports of `runSemgrepScanner`/
/// `runGitleaksScanner`'s exact commands (`packages/dagger-module/src/index.ts`),
/// including the empty-output fallback (a clean run can leave the report file
/// empty/absent depending on scanner version, which [`parse_scanner_findings`]
/// needs *some* valid JSON to parse rather than nothing at all). `set -eu` means
/// a real scanner failure aborts before the fallback write — paired with
/// `--expect=ANY` on the `with-exec` that runs it (see
/// [`scanner_pipeline_prefix`]) so a real failure doesn't also fail the whole
/// `dagger core` call, just leaves the exit code to reflect it.
fn scanner_script(name: ScannerName) -> &'static str {
    match name {
        ScannerName::Semgrep => {
            "set -eu\n\
             semgrep scan --config \"$SEMGREP_CONFIG\" --json --output /tmp/semgrep.json /src\n\
             if [ ! -s /tmp/semgrep.json ]; then printf '{\"results\":[]}' > /tmp/semgrep.json; fi"
        }
        ScannerName::Gitleaks => {
            "set -eu\n\
             gitleaks detect --source=/src --report-format=json --report-path=/tmp/gitleaks.json --redact --exit-code=0\n\
             if [ ! -s /tmp/gitleaks.json ]; then printf '[]' > /tmp/gitleaks.json; fi"
        }
        ScannerName::CargoAudit => {
            "set -eu\n\
             cargo install cargo-audit --locked\n\
             cargo audit --json > /tmp/cargo-audit.json\n\
             if [ ! -s /tmp/cargo-audit.json ]; then printf '{\"vulnerabilities\":{\"list\":[]}}' > /tmp/cargo-audit.json; fi"
        }
    }
}

/// The `dagger core <chain>` prefix both [`scanner_json_pipeline_args`] and
/// [`scanner_exit_code_pipeline_args`] build on: pulls `scanner.image`, mounts
/// `source_dir` at `/src`, and runs [`scanner_script`] — written to a file and
/// executed as `sh <path>` rather than inlined into `with-exec --args`, not for
/// style but because it has to be: `dagger core`'s `--args` value is
/// comma/CSV-parsed, and this script's embedded `"` characters (needed for the
/// JSON fallback content) broke that parser — verified for real (`invalid
/// argument ... parse error ... bare " in non-quoted-field`) before switching to
/// `with-new-file` + `sh <path>`, which sidesteps the CSV parsing entirely.
fn scanner_pipeline_prefix(source_dir: &str, scanner: &ScannerConfig) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "container".into(),
        "from".into(),
        format!("--address={}", scanner.image),
        "with-mounted-directory".into(),
        "--path=/src".into(),
        format!("--source={source_dir}"),
        "with-workdir".into(),
        "--path=/src".into(),
    ];
    if scanner.name == ScannerName::Semgrep {
        args.extend([
            "with-env-variable".into(),
            "--name=SEMGREP_CONFIG".into(),
            "--value=auto".into(),
        ]);
    }
    args.extend([
        "with-new-file".into(),
        "--path=/scan.sh".into(),
        format!("--contents={}", scanner_script(scanner.name)),
        "with-exec".into(),
        "--expect=ANY".into(),
        "--args=sh,/scan.sh".into(),
    ]);
    args
}

/// Builds the `dagger core <chain>` argument list that runs `scanner` and
/// returns its report file's contents (via `paws_dagger::core`, whose return
/// value is exactly what [`parse_scanner_findings`] expects to parse) — verified
/// for real against both scanners with genuine findings (a `semgrep`
/// `eval()`-detected finding, a `gitleaks` high-entropy secret finding), not
/// just the empty/no-findings case.
pub fn scanner_json_pipeline_args(source_dir: &str, scanner: &ScannerConfig) -> Vec<String> {
    let mut args = scanner_pipeline_prefix(source_dir, scanner);
    args.extend([
        "file".into(),
        format!("--path={}", scanner_output_path(scanner.name)),
        "contents".into(),
    ]);
    args
}

/// Same prefix as [`scanner_json_pipeline_args`] (replays from Dagger's own
/// cache rather than re-running the scan), but returns the scan command's exit
/// code instead — [`normalize_scanner_status`] needs both the exit code and the
/// findings count, and `dagger core`'s chains are strictly linear (`exit-code`
/// and `file`/`contents` are two different terminal calls on the same
/// `with-exec`'d container, so getting both needs two invocations, not one).
pub fn scanner_exit_code_pipeline_args(source_dir: &str, scanner: &ScannerConfig) -> Vec<String> {
    let mut args = scanner_pipeline_prefix(source_dir, scanner);
    args.push("exit-code".into());
    args
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
    let failed = scanners
        .iter()
        .filter(|s| s.status == ScannerStatus::Failed)
        .count();
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
    let runnable: Vec<&AuditScannerResult> = scanners
        .iter()
        .filter(|s| s.status != ScannerStatus::Skipped)
        .collect();

    let total_findings: usize = runnable
        .iter()
        .filter(|s| matches!(s.status, ScannerStatus::Findings | ScannerStatus::Pass))
        .map(|s| s.findings_count)
        .sum();

    let mut top_findings: Vec<TopFinding> = runnable
        .iter()
        .flat_map(|s| s.top_findings.clone())
        .collect();
    top_findings.sort_by(compare_top_findings);
    top_findings.truncate(10);

    let overall_status = derive_overall_status(&runnable.into_iter().cloned().collect::<Vec<_>>());

    AuditSummary {
        overall_status,
        detected_families: detection
            .families
            .iter()
            .map(|f| f.family.as_str().to_string())
            .collect(),
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
        summary
            .detected_families
            .iter()
            .filter(|f| f.as_str() != "generic")
            .count() as i64,
    );
    result.insert("fallbackMode".to_string(), i64::from(summary.fallback_mode));
    result.insert(
        "scannerFailureCount".to_string(),
        summary
            .scanners
            .iter()
            .filter(|s| s.status == ScannerStatus::Failed)
            .count() as i64,
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
        .map(|f| {
            if f == "generic" {
                "generic (baseline)".to_string()
            } else {
                f.clone()
            }
        })
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

    let failed_count = summary
        .scanners
        .iter()
        .filter(|s| s.status == ScannerStatus::Failed)
        .count();

    let mut lines = vec![
        "### Audit Intelligence".to_string(),
        String::new(),
        "| Field | Value |".to_string(),
        "| --- | --- |".to_string(),
        format!("| Overall status | `{:?}` |", summary.overall_status),
        format!(
            "| Detection confidence | `{:?}` |",
            summary.detection_confidence
        ),
        format!(
            "| Detected families | {} |",
            if families.is_empty() {
                "_(none)_".to_string()
            } else {
                format!("`{families}`")
            }
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

    lines
        .into_iter()
        .filter(|l| !l.is_empty() || true)
        .collect::<Vec<_>>()
        .join("\n")
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
    let parsed: serde_json::Value =
        serde_json::from_str(raw_json).unwrap_or(serde_json::Value::Null);
    let results = parsed
        .get("results")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

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
                    .or_else(|| {
                        extra
                            .and_then(|e| e.get("check_id"))
                            .and_then(|v| v.as_str())
                    })
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
    let parsed: serde_json::Value =
        serde_json::from_str(raw_json).unwrap_or(serde_json::Value::Null);
    let results = parsed.as_array().cloned().unwrap_or_default();

    let str_field = |v: &serde_json::Value, keys: &[&str]| -> Option<String> {
        keys.iter()
            .find_map(|k| v.get(k).and_then(|x| x.as_str()).map(String::from))
    };
    let num_field = |v: &serde_json::Value, keys: &[&str]| -> Option<u32> {
        keys.iter()
            .find_map(|k| v.get(k).and_then(|x| x.as_u64()).map(|n| n as u32))
    };

    let mut top_findings: Vec<TopFinding> = results
        .iter()
        .filter_map(|result| {
            let path = str_field(result, &["File", "file", "Path"])?;
            Some(TopFinding {
                rule: str_field(result, &["RuleID", "ruleID", "rule_id"])
                    .unwrap_or_else(|| "gitleaks-rule".to_string()),
                severity: normalize_severity(
                    str_field(result, &["Severity", "severity"])
                        .as_deref()
                        .or(Some("high")),
                ),
                path,
                line: num_field(result, &["StartLine", "startLine", "Line", "line"]),
                message: str_field(
                    result,
                    &["Description", "description", "Message", "message"],
                )
                .unwrap_or_else(|| "Gitleaks finding".to_string()),
                scanner: "gitleaks".to_string(),
            })
        })
        .collect();
    top_findings.sort_by(compare_top_findings);
    top_findings.truncate(10);

    (results.len(), top_findings)
}

/// `cargo audit --json`'s shape: `{"vulnerabilities":{"list":[{"advisory":{"id",
/// "title","severity"?},"package":{"name","version"}}, ...]}}`. RustSec
/// advisories rarely carry a plain `severity` string (CVSS scoring is optional
/// metadata most advisories don't set) — a known-vulnerable dependency
/// defaults to `High` rather than `normalize_severity`'s usual `Info` default,
/// so a real advisory can't be silently buried under genuinely low-signal
/// findings from other scanners in the same aggregated report (research.md R3).
fn parse_cargo_audit_findings(raw_json: &str) -> (usize, Vec<TopFinding>) {
    let parsed: serde_json::Value =
        serde_json::from_str(raw_json).unwrap_or(serde_json::Value::Null);
    let list = parsed
        .get("vulnerabilities")
        .and_then(|v| v.get("list"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut top_findings: Vec<TopFinding> = list
        .iter()
        .filter_map(|entry| {
            let advisory = entry.get("advisory")?;
            let package = entry.get("package");
            let package_name = package
                .and_then(|p| p.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown-crate");
            let package_version = package
                .and_then(|p| p.get("version"))
                .and_then(|v| v.as_str());

            Some(TopFinding {
                rule: advisory
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("RUSTSEC")
                    .to_string(),
                severity: match advisory.get("severity").and_then(|v| v.as_str()) {
                    Some(raw) => normalize_severity(Some(raw)),
                    None => Severity::High,
                },
                path: match package_version {
                    Some(version) => format!("{package_name}@{version}"),
                    None => package_name.to_string(),
                },
                line: None,
                message: advisory
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("cargo-audit finding")
                    .to_string(),
                scanner: "cargo-audit".to_string(),
            })
        })
        .collect();
    top_findings.sort_by(compare_top_findings);
    top_findings.truncate(10);

    (list.len(), top_findings)
}

/// Ported from `parseScannerFindings`.
pub fn parse_scanner_findings(scanner: ScannerName, raw_json: &str) -> (usize, Vec<TopFinding>) {
    match scanner {
        ScannerName::Semgrep => parse_semgrep_findings(raw_json),
        ScannerName::Gitleaks => parse_gitleaks_findings(raw_json),
        ScannerName::CargoAudit => parse_cargo_audit_findings(raw_json),
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
        let (findings_count, top_findings) =
            parse_scanner_findings(ScannerName::Semgrep, raw_semgrep);

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
        let gitleaks = scanners
            .iter()
            .find(|s| s.name == ScannerName::Gitleaks)
            .unwrap();
        assert!(!gitleaks.should_run);
    }

    #[test]
    fn cargo_audit_should_run_gated_on_rust_family_detection() {
        let with_rust = detect_language_families(&signals(&["Cargo.toml", "Cargo.lock"]));
        let scanners = select_audit_scanners(&with_rust, true);
        let cargo_audit = scanners
            .iter()
            .find(|s| s.name == ScannerName::CargoAudit)
            .unwrap();
        assert!(cargo_audit.should_run);
        assert_eq!(
            cargo_audit.family,
            ScannerFamily::Language(LanguageFamily::Rust)
        );

        let without_rust = detect_language_families(&signals(&["package.json"]));
        let scanners = select_audit_scanners(&without_rust, true);
        let cargo_audit = scanners
            .iter()
            .find(|s| s.name == ScannerName::CargoAudit)
            .unwrap();
        assert!(!cargo_audit.should_run);
    }

    #[test]
    fn parse_cargo_audit_findings_extracts_a_known_rustsec_advisory() {
        let raw = r#"{
            "vulnerabilities": {
                "found": true,
                "count": 1,
                "list": [
                    {
                        "advisory": {
                            "id": "RUSTSEC-2021-0001",
                            "title": "Integer overflow in example-crate",
                            "severity": "high"
                        },
                        "package": { "name": "example-crate", "version": "0.1.0" }
                    }
                ]
            }
        }"#;
        let (findings_count, top_findings) = parse_scanner_findings(ScannerName::CargoAudit, raw);
        assert_eq!(findings_count, 1);
        assert_eq!(top_findings.len(), 1);
        let finding = &top_findings[0];
        assert_eq!(finding.rule, "RUSTSEC-2021-0001");
        assert_eq!(finding.path, "example-crate@0.1.0");
        assert_eq!(finding.severity, Severity::High);
        assert_eq!(finding.message, "Integer overflow in example-crate");
        assert_eq!(finding.scanner, "cargo-audit");
    }

    #[test]
    fn parse_cargo_audit_findings_defaults_to_high_severity_when_advisory_omits_it() {
        // Most real RustSec advisories carry no CVSS/severity field at all
        // (research.md R3) — a known-vulnerable dependency must not be
        // silently demoted to `normalize_severity`'s usual `Info` default.
        let raw = r#"{"vulnerabilities":{"list":[{"advisory":{"id":"RUSTSEC-2022-9999","title":"t"},"package":{"name":"c","version":"1.0.0"}}]}}"#;
        let (_, top_findings) = parse_scanner_findings(ScannerName::CargoAudit, raw);
        assert_eq!(top_findings[0].severity, Severity::High);
    }

    #[test]
    fn parse_cargo_audit_findings_on_a_clean_report_produces_no_findings_and_does_not_affect_outcome()
     {
        let raw = r#"{"vulnerabilities":{"found":false,"count":0,"list":[]}}"#;
        let (findings_count, top_findings) = parse_scanner_findings(ScannerName::CargoAudit, raw);
        assert_eq!(findings_count, 0);
        assert!(top_findings.is_empty());

        let scanner_result = AuditScannerResult {
            name: "cargo-audit".to_string(),
            family: ScannerFamily::Language(LanguageFamily::Rust),
            status: normalize_scanner_status(Some(0), findings_count),
            findings_count,
            duration_ms: 50,
            failure_reason: None,
            top_findings,
        };
        let detection = detect_language_families(&signals(&["Cargo.toml", "Cargo.lock"]));
        let summary = aggregate_audit_results(&[scanner_result], &detection);
        assert_eq!(summary.overall_status, AuditOverallStatus::Pass);
        assert_eq!(summary.total_findings, 0);
    }

    fn scanner(name: ScannerName) -> ScannerConfig {
        select_audit_scanners(&detect_language_families(&signals(&["Cargo.toml"])), true)
            .into_iter()
            .find(|s| s.name == name)
            .unwrap()
    }

    #[test]
    fn semgrep_json_pipeline_pulls_the_registry_image_and_reads_its_report() {
        let args = scanner_json_pipeline_args("/host/src", &scanner(ScannerName::Semgrep));
        assert_eq!(args[0], "container");
        assert_eq!(args[2], "--address=returntocorp/semgrep:1.81.0");
        assert!(args.contains(&"--source=/host/src".to_string()));
        assert!(args.contains(&"--name=SEMGREP_CONFIG".to_string()));
        assert_eq!(args[args.len() - 3], "file");
        assert_eq!(args[args.len() - 2], "--path=/tmp/semgrep.json");
        assert_eq!(args.last(), Some(&"contents".to_string()));
    }

    #[test]
    fn gitleaks_json_pipeline_has_no_semgrep_config_env() {
        let args = scanner_json_pipeline_args("/host/src", &scanner(ScannerName::Gitleaks));
        assert_eq!(args[2], "--address=zricethezav/gitleaks:v8.24.2");
        assert!(!args.iter().any(|a| a == "--name=SEMGREP_CONFIG"));
        assert_eq!(args[args.len() - 2], "--path=/tmp/gitleaks.json");
    }

    #[test]
    fn exit_code_pipeline_shares_the_json_pipelines_prefix() {
        let scanner = scanner(ScannerName::Semgrep);
        let json_args = scanner_json_pipeline_args("/host/src", &scanner);
        let exit_args = scanner_exit_code_pipeline_args("/host/src", &scanner);

        // Identical up to the terminal call - same build, so Dagger's own
        // cache makes the second invocation replay instead of re-scanning.
        let json_prefix = &json_args[..json_args.len() - 3];
        let exit_prefix = &exit_args[..exit_args.len() - 1];
        assert_eq!(json_prefix, exit_prefix);
        assert_eq!(exit_args.last(), Some(&"exit-code".to_string()));
    }

    #[test]
    fn scanner_scripts_never_embed_a_raw_newline_inside_one_with_exec_args_token() {
        // dagger core's --args value is comma/CSV-parsed - an embedded
        // newline inside a single comma-separated field gets silently
        // truncated (verified for real). The scanner scripts avoid this by
        // never going through --args at all (with-new-file + `sh <path>`
        // instead), but this pins the actual with-exec args token used stays
        // newline-free as a regression guard on that choice.
        for name in [
            ScannerName::Semgrep,
            ScannerName::Gitleaks,
            ScannerName::CargoAudit,
        ] {
            let args = scanner_json_pipeline_args("/host/src", &scanner(name));
            let exec_args_token = args
                .iter()
                .find(|a| a.starts_with("--args="))
                .expect("with-exec --args token");
            assert!(!exec_args_token.contains('\n'));
        }
    }
}
