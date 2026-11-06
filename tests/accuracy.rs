//! Accuracy harness: labeled fixtures per pattern, precision and recall.
//!
//! Each fixture under `tests/fixtures/<name>/labels.json` lists, per metric, the keys a
//! correct scan reports (`positive`) and keys it must not (`negative`). Reported keys that are
//! unlabeled count as false positives, so labels must stay complete.
//!
//! Run `cargo test --test accuracy -- --nocapture` to print the table.

use prune::patterns::{Finding, PatternType};
use prune::Analyzer;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Deserialize)]
struct Label {
    positive: Vec<String>,
    negative: Vec<String>,
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn scan(name: &str) -> (Vec<Finding>, BTreeMap<String, Label>) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    let labels: BTreeMap<String, Label> =
        serde_json::from_str(&std::fs::read_to_string(source.join("labels.json")).unwrap()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&source, dir.path());
    let mut analyzer = Analyzer::new(dir.path()).unwrap();
    (analyzer.scan().unwrap().findings, labels)
}

fn context_name(context: &Option<String>) -> Option<String> {
    context
        .as_ref()
        .and_then(|text| text.split_once(": ").map(|(_, name)| name.to_string()))
}

/// The keys a scan reports for one metric.
fn reported(metric: &str, findings: &[Finding]) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for finding in findings {
        match (metric, finding.pattern_type) {
            ("dead_exports", PatternType::DeadExports) => {
                for symbol in &finding.symbols {
                    keys.insert(format!("{}::{}", symbol.file, symbol.name));
                }
            }
            ("import_cycle", PatternType::ImportCycle) => {
                let kind = if finding.related_concepts.iter().any(|c| c == "cycle:type") {
                    "type"
                } else {
                    "value"
                };
                let mut files: Vec<&str> = finding.locations.iter().map(|l| l.file.as_str()).collect();
                files.sort();
                keys.insert(format!("{kind}:{}", files.join("+")));
            }
            ("god_file", PatternType::GodFile) => {
                for location in &finding.locations {
                    keys.insert(location.file.clone());
                }
            }
            ("shape_pairs", PatternType::InterfaceSprawl | PatternType::Lexicon) => {
                if finding.related_concepts.iter().any(|c| c.starts_with("glossary:")) {
                    continue;
                }
                let names: BTreeSet<String> = finding
                    .locations
                    .iter()
                    .filter_map(|location| context_name(&location.context))
                    .collect();
                let names: Vec<String> = names.into_iter().collect();
                for (index, left) in names.iter().enumerate() {
                    for right in &names[index + 1..] {
                        keys.insert(format!("{left}~{right}"));
                    }
                }
            }
            ("over_abstraction", PatternType::OverAbstraction) => {
                let name = finding.title.rsplit(": ").next().unwrap_or("");
                for location in &finding.locations {
                    keys.insert(format!("{}::{name}", location.file));
                }
            }
            ("lexicon_glossary", PatternType::Lexicon) => {
                for concept in &finding.related_concepts {
                    if concept.starts_with("glossary:") {
                        keys.insert(concept.clone());
                    }
                }
            }
            ("lexicon_smash", PatternType::Lexicon) if finding.title.contains("resource tokens") => {
                for location in &finding.locations {
                    if let Some(name) = context_name(&location.context) {
                        keys.insert(name);
                    }
                }
            }
            _ => {}
        }
    }
    keys
}

/// Normalize a pair key so `A~B` and `B~A` compare equal.
fn normalize(metric: &str, key: &str) -> String {
    if metric == "shape_pairs" {
        if let Some((left, right)) = key.split_once('~') {
            let (a, b) = if left <= right { (left, right) } else { (right, left) };
            return format!("{a}~{b}");
        }
    }
    key.to_string()
}

#[derive(Default, Clone, Copy)]
struct Score {
    true_positive: usize,
    false_positive: usize,
    false_negative: usize,
}

impl Score {
    fn precision(&self) -> f64 {
        let reported = self.true_positive + self.false_positive;
        if reported == 0 {
            1.0
        } else {
            self.true_positive as f64 / reported as f64
        }
    }

    fn recall(&self) -> f64 {
        let expected = self.true_positive + self.false_negative;
        if expected == 0 {
            1.0
        } else {
            self.true_positive as f64 / expected as f64
        }
    }
}

#[test]
fn accuracy_on_labeled_fixtures() {
    let mut totals: BTreeMap<String, Score> = BTreeMap::new();
    let mut misses: Vec<String> = Vec::new();
    for fixture in ["reachability", "shapes", "godfile", "static_site"] {
        let (findings, labels) = scan(fixture);
        for (metric, label) in &labels {
            let got: BTreeSet<String> = reported(metric, &findings)
                .iter()
                .map(|key| normalize(metric, key))
                .collect();
            let positive: BTreeSet<String> = label.positive.iter().map(|k| normalize(metric, k)).collect();
            let negative: BTreeSet<String> = label.negative.iter().map(|k| normalize(metric, k)).collect();
            let score = totals.entry(metric.clone()).or_default();
            for key in &got {
                if positive.contains(key) {
                    score.true_positive += 1;
                } else {
                    score.false_positive += 1;
                    let why = if negative.contains(key) { "labeled negative" } else { "unlabeled" };
                    misses.push(format!("{fixture}/{metric}: false positive {key} ({why})"));
                }
            }
            for key in positive.difference(&got) {
                score.false_negative += 1;
                misses.push(format!("{fixture}/{metric}: missed {key}"));
            }
        }
    }

    println!("\n{:<18} {:>4} {:>4} {:>4} {:>10} {:>8}", "metric", "tp", "fp", "fn", "precision", "recall");
    for (metric, score) in &totals {
        println!(
            "{:<18} {:>4} {:>4} {:>4} {:>9.0}% {:>7.0}%",
            metric,
            score.true_positive,
            score.false_positive,
            score.false_negative,
            score.precision() * 100.0,
            score.recall() * 100.0
        );
    }
    for miss in &misses {
        println!("  {miss}");
    }
    for (metric, score) in &totals {
        assert!(
            score.precision() >= 0.9 && score.recall() >= 0.9,
            "{metric}: precision {:.2} recall {:.2}\n{}",
            score.precision(),
            score.recall(),
            misses.join("\n")
        );
    }
}
