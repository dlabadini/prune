//! Pattern detection for anti-patterns in codebases

mod barrel_bloat;
mod dead_exports;
mod god_file;
mod import_cycle;
mod interface_sprawl;
mod over_abstraction;
mod redundant_logic;

pub use barrel_bloat::BarrelBloatDetector;
pub use dead_exports::DeadExportsDetector;
pub use god_file::GodFileDetector;
pub use import_cycle::ImportCycleDetector;
pub use interface_sprawl::InterfaceSprawlDetector;
pub use over_abstraction::OverAbstractionDetector;
pub use redundant_logic::RedundantLogicDetector;

use crate::config::Config;
use crate::db::Database;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Severity level for findings
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Low => "low",
            Severity::Medium => "medium",
            Severity::High => "high",
            Severity::Critical => "critical",
        }
    }

    pub fn color(&self) -> &'static str {
        match self {
            Severity::Low => "blue",
            Severity::Medium => "yellow",
            Severity::High => "red",
            Severity::Critical => "magenta",
        }
    }
}

/// A location in the codebase where an issue was found
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub file: String,
    pub start_line: usize,
    pub end_line: usize,
    pub snippet: Option<String>,
    pub context: Option<String>,
}

impl Location {
    pub fn new(file: String, start_line: usize, end_line: usize) -> Self {
        Self {
            file,
            start_line,
            end_line,
            snippet: None,
            context: None,
        }
    }

    pub fn with_snippet(mut self, snippet: String) -> Self {
        self.snippet = Some(snippet);
        self
    }

    pub fn with_context(mut self, context: String) -> Self {
        self.context = Some(context);
        self
    }
}

/// Scoring pool. Emerging findings can raise the repo score and cannot lower it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Essential,
    Recommended,
    Emerging,
}

impl Tier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Tier::Essential => "essential",
            Tier::Recommended => "recommended",
            Tier::Emerging => "emerging",
        }
    }
}

/// The type of pattern detected
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternType {
    InterfaceSprawl,
    RedundantLogic,
    GodFile,
    BarrelBloat,
    DeadExports,
    OverAbstraction,
    ImportCycle,
}

impl PatternType {
    pub fn as_str(&self) -> &'static str {
        match self {
            PatternType::InterfaceSprawl => "interface_sprawl",
            PatternType::RedundantLogic => "redundant_logic",
            PatternType::GodFile => "god_file",
            PatternType::BarrelBloat => "barrel_bloat",
            PatternType::DeadExports => "dead_exports",
            PatternType::OverAbstraction => "over_abstraction",
            PatternType::ImportCycle => "import_cycle",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            PatternType::InterfaceSprawl => "Interface Sprawl",
            PatternType::RedundantLogic => "Redundant Logic",
            PatternType::GodFile => "God File",
            PatternType::BarrelBloat => "Barrel Bloat",
            PatternType::DeadExports => "Dead Exports",
            PatternType::OverAbstraction => "Over-Abstraction",
            PatternType::ImportCycle => "Import Cycle",
        }
    }

    /// Pool this pattern belongs to. Emerging can only add bonus points.
    pub fn tier(&self) -> Tier {
        match self {
            PatternType::GodFile | PatternType::DeadExports | PatternType::ImportCycle => {
                Tier::Essential
            }
            PatternType::InterfaceSprawl
            | PatternType::RedundantLogic
            | PatternType::BarrelBloat => Tier::Recommended,
            PatternType::OverAbstraction => Tier::Emerging,
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            PatternType::InterfaceSprawl => "Multiple inline types representing the same concept",
            PatternType::RedundantLogic => {
                "Semantically similar code blocks that could be consolidated"
            }
            PatternType::GodFile => "File with too many exports or excessive complexity",
            PatternType::BarrelBloat => "Index file re-exporting too many modules",
            PatternType::DeadExports => "Exported symbols with no internal consumers",
            PatternType::OverAbstraction => "Layers of indirection with single implementations",
            PatternType::ImportCycle => "Modules that import each other",
        }
    }

    pub fn all() -> &'static [PatternType] {
        &[
            PatternType::InterfaceSprawl,
            PatternType::RedundantLogic,
            PatternType::GodFile,
            PatternType::BarrelBloat,
            PatternType::DeadExports,
            PatternType::OverAbstraction,
            PatternType::ImportCycle,
        ]
    }
}

/// A detected anti-pattern finding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    /// Unique identifier for this finding
    pub id: String,
    /// Type of pattern detected
    pub pattern_type: PatternType,
    /// Severity level
    pub severity: Severity,
    /// Confidence score (0.0-1.0)
    pub confidence: f32,
    /// Short title describing the issue
    pub title: String,
    /// Detailed description of the issue
    pub description: String,
    /// Why this matters for the codebase
    pub rationale: String,
    /// Locations in the codebase where this issue appears
    pub locations: Vec<Location>,
    /// Suggested approach for fixing
    pub suggested_fix: String,
    /// When this was first detected
    pub first_detected: DateTime<Utc>,
    /// When this was last seen
    pub last_seen: DateTime<Utc>,
    /// Related symbol names or concepts
    pub related_concepts: Vec<String>,
}

impl Finding {
    pub fn new(
        pattern_type: PatternType,
        severity: Severity,
        confidence: f32,
        title: String,
        description: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: format!(
                "{}-{}",
                pattern_type.as_str(),
                &Uuid::new_v4().to_string()[..8]
            ),
            pattern_type,
            severity,
            confidence,
            title,
            description,
            rationale: String::new(),
            locations: Vec::new(),
            suggested_fix: String::new(),
            first_detected: now,
            last_seen: now,
            related_concepts: Vec::new(),
        }
    }

    pub fn with_rationale(mut self, rationale: String) -> Self {
        self.rationale = rationale;
        self
    }

    pub fn with_locations(mut self, locations: Vec<Location>) -> Self {
        self.locations = locations;
        self
    }

    pub fn with_suggested_fix(mut self, fix: String) -> Self {
        self.suggested_fix = fix;
        self
    }

    pub fn with_related_concepts(mut self, concepts: Vec<String>) -> Self {
        self.related_concepts = concepts;
        self
    }

    pub fn add_location(&mut self, location: Location) {
        self.locations.push(location);
    }

    /// Short ID for display (first 8 chars)
    pub fn short_id(&self) -> &str {
        &self.id
    }

    /// Replace the per-scan uuid with an id derived from the pattern, title, and locations.
    pub fn stabilize_id(&mut self) {
        self.id = crate::score::stable_finding_id(self.pattern_type, &self.title, &self.locations);
    }
}

/// Trait for pattern detectors
pub trait PatternDetector: Send + Sync {
    /// The type of pattern this detector finds
    fn pattern_type(&self) -> PatternType;

    /// Detect patterns in the indexed codebase
    fn detect(&self, db: &Database, config: &Config) -> Vec<Finding>;

    /// Whether this detector is enabled in the config
    fn is_enabled(&self, config: &Config) -> bool {
        config.is_pattern_enabled(self.pattern_type().as_str())
    }
}

/// Run all enabled pattern detectors
pub fn run_all_detectors(db: &Database, config: &Config) -> Vec<Finding> {
    let detectors: Vec<Box<dyn PatternDetector>> = vec![
        Box::new(GodFileDetector),
        Box::new(BarrelBloatDetector),
        Box::new(DeadExportsDetector),
        Box::new(InterfaceSprawlDetector),
        Box::new(RedundantLogicDetector),
        Box::new(OverAbstractionDetector),
        Box::new(ImportCycleDetector),
    ];

    let mut all_findings = Vec::new();

    for detector in detectors {
        if detector.is_enabled(config) {
            let findings = detector.detect(db, config);
            all_findings.extend(findings);
        }
    }

    for finding in &mut all_findings {
        finding.stabilize_id();
    }

    // Sort by severity (highest first), then by confidence
    all_findings.sort_by(|a, b| {
        b.severity.cmp(&a.severity).then_with(|| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });

    all_findings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_severity_ordering() {
        assert!(Severity::Critical > Severity::High);
        assert!(Severity::High > Severity::Medium);
        assert!(Severity::Medium > Severity::Low);
    }

    #[test]
    fn test_finding_creation() {
        let finding = Finding::new(
            PatternType::GodFile,
            Severity::High,
            0.95,
            "God file detected".to_string(),
            "utils.ts has too many exports".to_string(),
        );
        assert!(!finding.id.is_empty());
        assert!(finding.id.starts_with("god_file-"));
    }
}
