//! Baselines and change scoping: report only what is new, or only what a change touches.

use crate::db::Database;
use crate::graph::ModuleGraph;
use crate::patterns::Finding;
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::path::Path;

/// Default baseline file, committed next to `.prunerc`.
pub const DEFAULT_BASELINE: &str = ".prune-baseline.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineEntry {
    pub id: String,
    pub pattern: String,
    pub title: String,
}

/// Findings accepted as they are; CI reports only findings missing from it.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Baseline {
    pub version: u32,
    pub findings: Vec<BaselineEntry>,
}

impl Baseline {
    pub fn from_findings(findings: &[Finding]) -> Self {
        let mut entries: Vec<BaselineEntry> = findings
            .iter()
            .map(|finding| BaselineEntry {
                id: finding.id.clone(),
                pattern: finding.pattern_type.as_str().to_string(),
                title: finding.title.clone(),
            })
            .collect();
        entries.sort_by(|a, b| a.id.cmp(&b.id));
        Self {
            version: 1,
            findings: entries,
        }
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        serde_json::from_str(&text).map_err(std::io::Error::other)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, text + "\n")
    }

    /// Findings whose id is not in the baseline.
    pub fn new_findings(&self, findings: Vec<Finding>) -> Vec<Finding> {
        let known: HashSet<&str> = self.findings.iter().map(|entry| entry.id.as_str()).collect();
        findings
            .into_iter()
            .filter(|finding| !known.contains(finding.id.as_str()))
            .collect()
    }
}

/// Files changed relative to `base` (committed, staged, unstaged, and untracked), repo-relative
/// to `root`.
pub fn changed_files(root: &Path, base: &str) -> std::io::Result<HashSet<String>> {
    let prefix = git(root, &["rev-parse", "--show-prefix"])?.trim().to_string();
    let mut files = HashSet::new();
    let diff = git(root, &["diff", "--name-only", "--relative", base])?;
    let untracked = git(root, &["ls-files", "--others", "--exclude-standard"])?;
    for line in diff.lines().chain(untracked.lines()) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        files.insert(line.strip_prefix(&prefix).unwrap_or(line).to_string());
    }
    Ok(files)
}

/// Files staged for commit (added, copied, modified, renamed), repo-relative to `root`.
pub fn staged_files(root: &Path) -> std::io::Result<HashSet<String>> {
    let prefix = git(root, &["rev-parse", "--show-prefix"])?.trim().to_string();
    let diff = git(
        root,
        &["diff", "--cached", "--name-only", "--relative", "--diff-filter=ACMR"],
    )?;
    Ok(diff
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.strip_prefix(&prefix).unwrap_or(line).to_string())
        .collect())
}

fn git(root: &Path, args: &[&str]) -> std::io::Result<String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// `files` plus every file that imports one of them, transitively.
pub fn with_dependents(db: &Database, files: &HashSet<String>) -> HashSet<String> {
    let Ok(graph) = ModuleGraph::load(db) else {
        return files.clone();
    };
    let mut importers: std::collections::HashMap<i64, Vec<i64>> = std::collections::HashMap::new();
    for edge in &graph.edges {
        if let Some(target) = edge.target_file_id {
            importers.entry(target).or_default().push(edge.source_file_id);
        }
    }
    let mut seen: HashSet<i64> = HashSet::new();
    let mut queue: VecDeque<i64> = files.iter().filter_map(|path| graph.file_id(path)).collect();
    while let Some(id) = queue.pop_front() {
        if !seen.insert(id) {
            continue;
        }
        for importer in importers.get(&id).into_iter().flatten() {
            queue.push_back(*importer);
        }
    }
    let mut out: HashSet<String> = seen.into_iter().map(|id| graph.path(id).to_string()).collect();
    out.extend(files.iter().cloned());
    out
}

/// `files` plus the files they import directly. A change to a file can leave exports in the
/// files it imports unused, so a baselined check looks one hop down as well.
pub fn with_imports(db: &Database, files: &HashSet<String>) -> HashSet<String> {
    let mut out = files.clone();
    let Ok(graph) = ModuleGraph::load(db) else {
        return out;
    };
    let ids: HashSet<i64> = files.iter().filter_map(|path| graph.file_id(path)).collect();
    for edge in &graph.edges {
        if let Some(target) = edge.target_file_id {
            if ids.contains(&edge.source_file_id) {
                out.insert(graph.path(target).to_string());
            }
        }
    }
    out
}

/// Findings with at least one location in `scope`.
pub fn touching(findings: Vec<Finding>, scope: &HashSet<String>) -> Vec<Finding> {
    findings
        .into_iter()
        .filter(|finding| {
            finding
                .locations
                .iter()
                .any(|location| scope.contains(&location.file))
                || finding.symbols.iter().any(|symbol| scope.contains(&symbol.file))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patterns::{Location, PatternType, Severity};

    fn finding(id: &str, file: &str) -> Finding {
        let mut finding = Finding::new(PatternType::GodFile, Severity::Low, 0.5, id.into(), "d".into())
            .with_locations(vec![Location::new(file.into(), 1, 1)]);
        finding.id = id.into();
        finding
    }

    #[test]
    fn baseline_hides_known_findings() {
        let baseline = Baseline::from_findings(&[finding("a", "x.ts")]);
        let fresh = baseline.new_findings(vec![finding("a", "x.ts"), finding("b", "y.ts")]);
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].id, "b");
    }

    #[test]
    fn touching_keeps_findings_in_scope() {
        let scope: HashSet<String> = ["y.ts".to_string()].into_iter().collect();
        let kept = touching(vec![finding("a", "x.ts"), finding("b", "y.ts")], &scope);
        assert_eq!(kept.len(), 1);
    }
}
