use serde::{Deserialize, Serialize};
use std::path::Path;

/// QC check severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum QcLevel {
    #[default]
    Info,
    Warning,
    Error,
}

/// Single QC check result.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QcResult {
    pub level: QcLevel,
    pub code: String,
    pub message: String,
}

/// Full QC report for a DCP.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QcReport {
    pub dcp_path: String,
    pub passed: bool,
    pub results: Vec<QcResult>,
    pub error_count: usize,
    pub warning_count: usize,
    pub info_count: usize,
}

/// Run quality control checks on a DCP by delegating to dcpdoctor-core verify.
pub fn run_qc(dcp_dir: &Path) -> QcReport {
    let mut report = QcReport {
        dcp_path: dcp_dir.to_string_lossy().into_owned(),
        ..Default::default()
    };

    if !dcp_dir.exists() {
        report.results.push(QcResult {
            level: QcLevel::Error,
            code: "DCP_NOT_FOUND".into(),
            message: format!("DCP directory not found: {}", dcp_dir.display()),
        });
        report.error_count = 1;
        return report;
    }

    let verify_result = crate::verify::verify_dcp_for_report(dcp_dir);

    for err in &verify_result.errors {
        report.results.push(QcResult {
            level: QcLevel::Error,
            code: "VERIFY_ERROR".into(),
            message: err.clone(),
        });
    }

    for warn in &verify_result.warnings {
        report.results.push(QcResult {
            level: QcLevel::Warning,
            code: "VERIFY_WARNING".into(),
            message: warn.clone(),
        });
    }

    for note in &verify_result.info {
        report.results.push(QcResult {
            level: QcLevel::Info,
            code: "VERIFY_INFO".into(),
            message: note.clone(),
        });
    }

    report.error_count = verify_result.errors.len();
    report.warning_count = verify_result.warnings.len();
    report.info_count = verify_result.info.len();
    report.passed = verify_result.valid;

    // Additional structural checks
    let assetmap_exists =
        dcp_dir.join("ASSETMAP").exists() || dcp_dir.join("ASSETMAP.xml").exists();
    if !assetmap_exists {
        report.results.push(QcResult {
            level: QcLevel::Error,
            code: "MISSING_ASSETMAP".into(),
            message: "ASSETMAP not found".into(),
        });
        report.error_count += 1;
        report.passed = false;
    }

    let volindex_exists =
        dcp_dir.join("VOLINDEX").exists() || dcp_dir.join("VOLINDEX.xml").exists();
    if !volindex_exists {
        report.results.push(QcResult {
            level: QcLevel::Warning,
            code: "MISSING_VOLINDEX".into(),
            message: "VOLINDEX not found".into(),
        });
        report.warning_count += 1;
    }

    report
}
