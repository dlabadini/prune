//! Pattern detection for anti-patterns in codebases

mod barrel_bloat;
mod dead_exports;
mod god_file;
mod import_cycle;
pub mod interface_sprawl;
mod lexicon;
mod over_abstraction;
mod redundant_logic;

pub use barrel_bloat::BarrelBloatDetector;
pub use dead_exports::DeadExportsDetector;
pub use god_file::GodFileDetector;
pub use import_cycle::ImportCycleDetector;
pub use interface_sprawl::InterfaceSprawlDetector;
pub use lexicon::LexiconDetector;
pub use over_abstraction::OverAbstractionDetector;
pub use redundant_logic::RedundantLogicDetector;

use crate::config::Config;
use crate::db::Database;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
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

/// One symbol a finding is about, with how it is used.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FindingSymbol {
    pub name: String,
    pub kind: String,
    pub file: String,
    pub line: usize,
    /// `used`, `test_only`, `same_file_only`, `unused`, `unreachable`
    pub usage: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
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
    Lexicon,
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
            PatternType::Lexicon => "lexicon",
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
            PatternType::Lexicon => "Lexicon",
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
            | PatternType::Lexicon
            | PatternType::RedundantLogic
            | PatternType::BarrelBloat => Tier::Recommended,
            PatternType::OverAbstraction => Tier::Emerging,
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            PatternType::InterfaceSprawl => {
                "Duplicate or partially overlapping types that should compose shared bundles"
            }
            PatternType::Lexicon => {
                "Naming drift and resource smash on shared shapes — align lexicon, compose facts"
            }
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
            PatternType::Lexicon,
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
    /// One-line machine/human summary of the pruning action
    #[serde(default)]
    pub action_summary: String,
    /// Structured compose/rename plan for agents
    #[serde(default)]
    pub prune_action: Option<crate::compose::PruneAction>,
    /// Symbols this finding is about, each classified by usage
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<FindingSymbol>,
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
            action_summary: String::new(),
            prune_action: None,
            symbols: Vec::new(),
        }
    }

    pub fn with_symbols(mut self, symbols: Vec<FindingSymbol>) -> Self {
        self.symbols = symbols;
        self
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

    pub fn with_action_summary(mut self, summary: String) -> Self {
        self.action_summary = summary;
        self
    }

    pub fn with_prune_action(mut self, action: crate::compose::PruneAction) -> Self {
        self.prune_action = Some(action);
        self
    }

    pub fn add_location(&mut self, location: Location) {
        self.locations.push(location);
    }

    /// Short ID for display (first 8 chars)
    pub fn short_id(&self) -> &str {
        &self.id
    }

    /// Replace the per-scan uuid with an id derived from the pattern and what it is about:
    /// the involved symbols, else the concepts and files. Titles, counts, and line numbers
    /// are left out so the id survives unrelated edits.
    pub fn stabilize_id(&mut self) {
        self.id = crate::score::stable_finding_id(self.pattern_type, &self.identity_keys());
    }

    /// The facts that make this finding the same finding across scans.
    pub fn identity_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = if self.symbols.is_empty() {
            let mut keys: Vec<String> = self
                .related_concepts
                .iter()
                .filter(|concept| !concept.starts_with("git:"))
                .map(|concept| format!("concept:{concept}"))
                .collect();
            keys.extend(
                self.locations
                    .iter()
                    .map(|location| format!("file:{}", location.file)),
            );
            keys
        } else {
            self.symbols
                .iter()
                .map(|symbol| format!("symbol:{}::{}", symbol.file, symbol.name))
                .collect()
        };
        keys.sort();
        keys.dedup();
        keys
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
        Box::new(LexiconDetector),
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

    let mut seen: HashMap<String, usize> = HashMap::new();
    for finding in &mut all_findings {
        finding.stabilize_id();
        let count = seen.entry(finding.id.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            finding.id = format!("{}-{}", finding.id, count);
        }
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

/// 64-bit fingerprint of a normalized body. Similar text shares bits.
pub(crate) fn simhash(text: &str) -> u64 {
    let mut weights = [0i32; 64];
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        line.hash(&mut hasher);
        let hashed = hasher.finish();
        for (bit, weight) in weights.iter_mut().enumerate() {
            if (hashed >> bit) & 1 == 1 {
                *weight += 1;
            } else {
                *weight -= 1;
            }
        }
    }
    let mut out = 0u64;
    for (bit, weight) in weights.iter().enumerate() {
        if *weight > 0 {
            out |= 1 << bit;
        }
    }
    out
}

/// Pairs that share a 16-bit band of `simhash`. This stays small on a real repo.
pub(crate) fn candidate_pairs(hashes: &[u64]) -> Vec<(usize, usize)> {
    let mut bands: HashMap<(u8, u16), Vec<usize>> = HashMap::new();
    for (index, hash) in hashes.iter().enumerate() {
        for band in 0..4u8 {
            let piece = ((hash >> (band * 16)) & 0xffff) as u16;
            bands.entry((band, piece)).or_default().push(index);
        }
    }

    let mut pairs = HashSet::new();
    for group in bands.values() {
        if group.len() < 2 {
            continue;
        }
        if group.len() > 48 {
            let mut exact: HashMap<u64, Vec<usize>> = HashMap::new();
            for &index in group {
                exact.entry(hashes[index]).or_default().push(index);
            }
            for ids in exact.values() {
                push_pairs(&mut pairs, ids);
            }
        } else {
            push_pairs(&mut pairs, group);
        }
    }
    pairs.into_iter().collect()
}

fn push_pairs(pairs: &mut HashSet<(usize, usize)>, ids: &[usize]) {
    for (left, a) in ids.iter().enumerate() {
        for b in ids.iter().skip(left + 1) {
            let (lo, hi) = if a < b { (*a, *b) } else { (*b, *a) };
            if lo != hi {
                pairs.insert((lo, hi));
            }
        }
    }
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
