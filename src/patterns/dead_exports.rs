//! Dead code by reachability: unreachable files, and exports nothing reachable uses.
//!
//! Each export is classified (see `graph::usage::Usage`): used, used only by tests, used only
//! inside its own file (drop `export`), unused, or in a file no entry reaches.

use super::{Finding, FindingSymbol, Location, PatternDetector, PatternType, Severity};
use crate::config::Config;
use crate::db::{Database, FileRole, Symbol};
use crate::graph::usage::{Reach, Usage, UsageReport};
use std::collections::BTreeMap;

pub struct DeadExportsDetector;

impl PatternDetector for DeadExportsDetector {
    fn pattern_type(&self) -> PatternType {
        PatternType::DeadExports
    }

    fn detect(&self, db: &Database, config: &Config) -> Vec<Finding> {
        let Ok(report) = UsageReport::load(db) else {
            return Vec::new();
        };
        detect_with(&report, config)
    }
}

/// Findings from an already-loaded usage report.
pub fn detect_with(report: &UsageReport, config: &Config) -> Vec<Finding> {
    let graph = &report.graph;
    let min_per_file = config.effective_dead_exports_min_per_file();
    let options = &config.dead_exports;
    let mut findings = Vec::new();

    let mut by_file: BTreeMap<&str, (i64, Vec<&Symbol>)> = BTreeMap::new();
    for file in graph.files.values() {
        if file.role == FileRole::Source {
            by_file.insert(file.path.as_str(), (file.id, Vec::new()));
        }
    }
    for symbol in graph.symbols.values().filter(|symbol| symbol.is_exported) {
        if let Some((_, symbols)) = by_file.get_mut(graph.path(symbol.file_id)) {
            symbols.push(symbol);
        }
    }

    for (path, (file_id, mut symbols)) in by_file {
        symbols.sort_by_key(|symbol| (symbol.start_line, symbol.name.clone()));
        match report.reach(file_id) {
            Reach::Unreachable => {
                if options.report_unreachable_files && !config.excludes_path(path) {
                    findings.push(unreachable_file(report, path, file_id, &symbols));
                }
            }
            // Test support code: reached only from tests, which is its job.
            Reach::TestOnly => {}
            Reach::Product => {
                let dead: Vec<(&Symbol, Usage)> = symbols
                    .iter()
                    .filter_map(|symbol| {
                        let usage = report.usage(symbol.id)?;
                        let reported = match usage {
                            Usage::Unused => true,
                            Usage::TestOnly => options.report_test_only,
                            Usage::SameFileOnly => options.report_same_file,
                            Usage::Used | Usage::Unreachable => false,
                        };
                        reported.then_some((*symbol, usage))
                    })
                    .collect();
                if dead.len() >= min_per_file && !dead.is_empty() {
                    findings.push(dead_exports_in_file(report, path, &dead));
                }
            }
        }
    }
    findings
}

fn symbol_entry(report: &UsageReport, path: &str, symbol: &Symbol, usage: Usage) -> FindingSymbol {
    let detail = if report.is_convex_function(symbol.id) {
        let internal = symbol
            .callee
            .as_deref()
            .is_some_and(|callee| callee.to_ascii_lowercase().contains("internal"));
        Some(if internal {
            "Convex internal function: no api/internal reference, string reference, or script uses it".to_string()
        } else {
            "Convex public function: no reference in this repo (external clients may still call it)".to_string()
        })
    } else {
        None
    };
    FindingSymbol {
        name: symbol.name.clone(),
        kind: symbol.kind.as_str().to_string(),
        file: path.to_string(),
        line: symbol.start_line,
        usage: usage.as_str().to_string(),
        detail,
    }
}

fn unreachable_file(report: &UsageReport, path: &str, file_id: i64, symbols: &[&Symbol]) -> Finding {
    let line_count = report
        .graph
        .files
        .get(&file_id)
        .map(|file| file.line_count)
        .unwrap_or(1)
        .max(1);
    let entries: Vec<FindingSymbol> = symbols
        .iter()
        .map(|symbol| symbol_entry(report, path, symbol, Usage::Unreachable))
        .collect();
    let importers: Vec<String> = report
        .graph
        .edges
        .iter()
        .filter(|edge| edge.target_file_id == Some(file_id))
        .map(|edge| edge.source_path.clone())
        .collect();
    let description = if importers.is_empty() {
        format!(
            "No entry point (package entry, script, config, framework route, Convex module) imports `{path}`, \
             directly or through other files."
        )
    } else {
        format!(
            "`{path}` is imported only by files that are themselves unreachable: {}.",
            importers.join(", ")
        )
    };
    let mut finding = Finding::new(
        PatternType::DeadExports,
        if symbols.len() > 5 || line_count > 200 {
            Severity::Medium
        } else {
            Severity::Low
        },
        0.8,
        format!("Unreachable file: {}", file_name(path)),
        description,
    )
    .with_rationale(
        "Code nothing loads still costs reading, type-checking, and refactoring time, and it misleads \
         search. If a tool or framework loads it, add the path to `[entries] files` in .prunerc."
            .to_string(),
    )
    .with_locations(vec![Location::new(path.to_string(), 1, line_count)
        .with_context("unreachable file".to_string())])
    .with_suggested_fix(format!(
        "1. Confirm nothing loads `{path}` (dynamic paths, CLI tools, framework conventions).\n\
         2. Delete the file, or register it under `[entries] files` in .prunerc if something does."
    ))
    .with_related_concepts(vec![path.to_string()])
    .with_action_summary(format!("delete unreachable file {path}"));
    finding.symbols = entries;
    finding.symbols.push(FindingSymbol {
        name: "*".to_string(),
        kind: "file".to_string(),
        file: path.to_string(),
        line: 1,
        usage: Usage::Unreachable.as_str().to_string(),
        detail: None,
    });
    finding
}

fn dead_exports_in_file(report: &UsageReport, path: &str, dead: &[(&Symbol, Usage)]) -> Finding {
    let count = |wanted: Usage| dead.iter().filter(|(_, usage)| *usage == wanted).count();
    let unused = count(Usage::Unused);
    let test_only = count(Usage::TestOnly);
    let same_file = count(Usage::SameFileOnly);
    let convex = dead
        .iter()
        .filter(|(symbol, usage)| *usage == Usage::Unused && report.is_convex_function(symbol.id))
        .count();

    let mut parts = Vec::new();
    if unused > 0 {
        parts.push(if convex == unused {
            format!("{unused} unreferenced Convex function{}", plural(unused))
        } else {
            format!("{unused} unused")
        });
    }
    if test_only > 0 {
        parts.push(format!("{test_only} test-only"));
    }
    if same_file > 0 {
        parts.push(format!("{same_file} needlessly exported"));
    }
    let title = format!("{} in {}", parts.join(", "), file_name(path));

    let group = |wanted: Usage| -> String {
        dead.iter()
            .filter(|(_, usage)| *usage == wanted)
            .map(|(symbol, _)| format!("`{}`", symbol.name))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut lines = Vec::new();
    if unused > 0 {
        lines.push(format!("Unused (nothing reachable imports or references them): {}.", group(Usage::Unused)));
    }
    if test_only > 0 {
        lines.push(format!("Used only by tests: {}.", group(Usage::TestOnly)));
    }
    if same_file > 0 {
        lines.push(format!(
            "Used only inside this file, so `export` is unnecessary: {}.",
            group(Usage::SameFileOnly)
        ));
    }

    let severity = if unused > 10 {
        Severity::High
    } else if unused > 3 {
        Severity::Medium
    } else {
        Severity::Low
    };

    let locations = dead
        .iter()
        .map(|(symbol, usage)| {
            Location::new(path.to_string(), symbol.start_line, symbol.end_line).with_context(format!(
                "{}: {} ({})",
                symbol.kind.as_str(),
                symbol.name,
                usage.label()
            ))
        })
        .collect();
    let symbols: Vec<FindingSymbol> = dead
        .iter()
        .map(|(symbol, usage)| symbol_entry(report, path, symbol, *usage))
        .collect();
    let names: Vec<String> = dead.iter().map(|(symbol, _)| symbol.name.clone()).collect();

    Finding::new(
        PatternType::DeadExports,
        severity,
        if unused > 0 { 0.9 } else { 0.8 },
        title,
        lines.join("\n"),
    )
    .with_rationale(
        "Exports nothing reachable uses widen the API surface, survive refactors, and mislead readers. \
         Exports used only inside their file should drop `export`; test-only exports usually mean \
         the tested code path is itself dead."
            .to_string(),
    )
    .with_locations(locations)
    .with_suggested_fix(generate_fix_suggestion(path, dead))
    .with_related_concepts(names)
    .with_action_summary(format!("prune {} export{} in {path}", dead.len(), plural(dead.len())))
    .with_symbols(symbols)
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn generate_fix_suggestion(file_path: &str, dead: &[(&Symbol, Usage)]) -> String {
    let mut steps = vec![format!("In `{file_path}`:")];
    for (symbol, usage) in dead {
        let action = match usage {
            Usage::Unused => "delete it (or drop `export` if the file still needs it)",
            Usage::TestOnly => "delete it with its tests, or move it into the test helpers",
            Usage::SameFileOnly => "remove the `export` keyword",
            Usage::Used | Usage::Unreachable => "keep",
        };
        steps.push(format!("- `{}` ({}): {action}", symbol.name, usage.label()));
    }
    steps.push(
        "\nIf a tool or framework loads one of these by name, add its file to `[entries] files` in .prunerc."
            .to_string(),
    );
    steps.join("\n")
}
