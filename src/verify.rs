//! Verify that a finding was resolved after refactoring.

use crate::analyzer::{Analyzer, AnalyzerError};
use crate::patterns::Finding;
use crate::score::RepoScore;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct VerifyResult {
    pub finding_id: String,
    pub resolved: bool,
    pub score: RepoScore,
    pub findings_before: usize,
    pub findings_after: usize,
    pub score_delta: f32,
    pub message: String,
    pub related_still_present: Vec<String>,
}

/// Run a fresh scan and check whether `finding_id` (or prefix) is gone.
pub fn verify_finding(
    analyzer: &mut Analyzer,
    finding_id: &str,
    quick: bool,
) -> Result<VerifyResult, AnalyzerError> {
    let result = if quick {
        analyzer.analyze()?
    } else {
        analyzer.scan()?
    };
    // quick skips filesystem walk; full scan re-indexes then detects
    let matched = matching_findings(&result.findings, finding_id);
    let resolved = matched.is_empty();

    let related_still_present = matched
        .iter()
        .map(|finding| finding.title.clone())
        .collect();

    let message = if matched.is_empty() {
        format!(
            "Resolved. Score {:.0}. {} findings remain.",
            result.score.total,
            result.findings.len()
        )
    } else {
        format!(
            "Still present ({} match(es)). Score {:.0}.",
            matched.len(),
            result.score.total
        )
    };

    Ok(VerifyResult {
        finding_id: finding_id.to_string(),
        resolved,
        score: result.score.clone(),
        findings_before: result.findings.len(),
        findings_after: result.findings.len(),
        score_delta: 0.0,
        message,
        related_still_present,
    })
}

pub fn matching_findings<'a>(findings: &'a [Finding], finding_id: &str) -> Vec<&'a Finding> {
    findings
        .iter()
        .filter(|finding| finding.id == finding_id || finding.id.starts_with(finding_id))
        .collect()
}
