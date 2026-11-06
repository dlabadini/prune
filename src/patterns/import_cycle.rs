//! Import cycles from the resolved module graph.

use super::{Finding, Location, PatternDetector, PatternType, Severity};
use crate::config::Config;
use crate::analyzer::parser::ImportKind;
use crate::db::{Database, FileRole};
use crate::graph::ModuleGraph;
use petgraph::algo::tarjan_scc;
use petgraph::graph::DiGraph;
use std::collections::{HashMap, HashSet};

pub struct ImportCycleDetector;

/// Whether a cycle orders module initialization or exists only in the type system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CycleKind {
    Value,
    Type,
}

impl PatternDetector for ImportCycleDetector {
    fn pattern_type(&self) -> PatternType {
        PatternType::ImportCycle
    }

    fn detect(&self, db: &Database, _config: &Config) -> Vec<Finding> {
        let Ok(graph) = ModuleGraph::load(db) else {
            return Vec::new();
        };
        let source = |id: i64| graph.role(id) == FileRole::Source;

        // Value graph: edges that run at load time. Full graph adds type-only edges.
        let mut value_edges = Vec::new();
        let mut all_edges = Vec::new();
        let mut self_loops = Vec::new();
        for edge in &graph.edges {
            let Some(target) = edge.target_file_id else {
                continue;
            };
            if !source(edge.source_file_id) || !source(target) {
                continue;
            }
            if matches!(edge.kind, ImportKind::Dynamic | ImportKind::Path) {
                continue;
            }
            if edge.source_file_id == target {
                if graph.edge_is_value(edge) {
                    self_loops.push(edge.source_path.clone());
                }
                continue;
            }
            all_edges.push((edge.source_file_id, target));
            if graph.edge_is_value(edge) {
                value_edges.push((edge.source_file_id, target));
            }
        }

        let mut findings = Vec::new();
        let value_components = components(&value_edges);
        let in_value_cycle: HashSet<i64> = value_components.iter().flatten().copied().collect();
        for component in &value_components {
            let files = paths(&graph, component);
            findings.push(cycle_finding(&files, CycleKind::Value));
        }
        for component in components(&all_edges) {
            // A component that contains a value cycle is reported as that value cycle; only
            // components held together by type edges alone are type-only cycles.
            if component.iter().any(|id| in_value_cycle.contains(id)) {
                continue;
            }
            let files = paths(&graph, &component);
            findings.push(cycle_finding(&files, CycleKind::Type));
        }
        self_loops.sort();
        self_loops.dedup();
        for path in self_loops {
            findings.push(cycle_finding(&[path], CycleKind::Value));
        }
        findings
    }
}

/// Strongly connected components with more than one file.
fn components(edges: &[(i64, i64)]) -> Vec<Vec<i64>> {
    let mut graph: DiGraph<i64, ()> = DiGraph::new();
    let mut nodes: HashMap<i64, petgraph::graph::NodeIndex> = HashMap::new();
    for (from, to) in edges {
        let a = *nodes.entry(*from).or_insert_with(|| graph.add_node(*from));
        let b = *nodes.entry(*to).or_insert_with(|| graph.add_node(*to));
        graph.add_edge(a, b, ());
    }
    tarjan_scc(&graph)
        .into_iter()
        .filter(|component| component.len() > 1)
        .map(|component| {
            let mut ids: Vec<i64> = component.into_iter().map(|node| graph[node]).collect();
            ids.sort_unstable();
            ids
        })
        .collect()
}

fn paths(graph: &ModuleGraph, ids: &[i64]) -> Vec<String> {
    let mut files: Vec<String> = ids.iter().map(|id| graph.path(*id).to_string()).collect();
    files.sort();
    files
}

fn cycle_finding(files: &[String], kind: CycleKind) -> Finding {
    let title = match (files.len(), kind) {
        (1, _) => format!("Import cycle: {} imports itself", file_name(&files[0])),
        (count, CycleKind::Value) => format!("Import cycle across {count} files"),
        (count, CycleKind::Type) => format!("Type-only import cycle across {count} files"),
    };
    let description = match kind {
        CycleKind::Value => format!(
            "These modules import each other at runtime: {}. Cycles make initialization order fragile and hide unused code.",
            files.join(" → ")
        ),
        CycleKind::Type => format!(
            "These modules reference each other only through types: {}. Nothing breaks at runtime, \
             but the modules cannot be understood or moved independently.",
            files.join(" → ")
        ),
    };
    let locations = files
        .iter()
        .map(|path| {
            Location::new(path.clone(), 1, 1).with_context("Part of an import cycle".to_string())
        })
        .collect();
    let severity = match kind {
        CycleKind::Type => Severity::Low,
        CycleKind::Value if files.len() > 3 => Severity::High,
        CycleKind::Value => Severity::Medium,
    };
    let mut concepts = files.to_vec();
    concepts.push(match kind {
        CycleKind::Value => "cycle:value".to_string(),
        CycleKind::Type => "cycle:type".to_string(),
    });
    Finding::new(PatternType::ImportCycle, severity, 0.9, title, description)
    .with_rationale(
        "Import cycles force modules to load before their dependencies are finished. \
         They also defeat tree-shaking and make dead exports harder to see."
            .to_string(),
    )
    .with_locations(locations)
    .with_suggested_fix(
        "1. Pick one direction for the dependency.\n\
         2. Move the shared types or functions into a third module that both can import.\n\
         3. Replace the back-edge with a direct import from that shared module."
            .to_string(),
    )
    .with_related_concepts(concepts)
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
