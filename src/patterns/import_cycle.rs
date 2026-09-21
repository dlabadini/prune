//! Import cycles from the resolved module graph.

use super::{Finding, Location, PatternDetector, PatternType, Severity};
use crate::config::Config;
use crate::db::Database;
use petgraph::algo::tarjan_scc;
use petgraph::graph::DiGraph;
use std::collections::HashMap;

pub struct ImportCycleDetector;

impl PatternDetector for ImportCycleDetector {
    fn pattern_type(&self) -> PatternType {
        PatternType::ImportCycle
    }

    fn detect(&self, db: &Database, _config: &Config) -> Vec<Finding> {
        let Ok(edges) = db.import_edges() else {
            return Vec::new();
        };

        let mut graph: DiGraph<String, ()> = DiGraph::new();
        let mut nodes: HashMap<String, petgraph::graph::NodeIndex> = HashMap::new();
        let mut self_loops = Vec::new();

        for edge in edges {
            let Some(target) = edge.target_path else {
                continue;
            };
            if edge.source_path == target {
                self_loops.push(edge.source_path);
                continue;
            }
            let source = *nodes
                .entry(edge.source_path.clone())
                .or_insert_with(|| graph.add_node(edge.source_path.clone()));
            let dest = *nodes
                .entry(target.clone())
                .or_insert_with(|| graph.add_node(target));
            graph.add_edge(source, dest, ());
        }

        let mut findings = Vec::new();
        for component in tarjan_scc(&graph) {
            if component.len() < 2 {
                continue;
            }
            let files: Vec<String> = component
                .into_iter()
                .filter_map(|node| graph.node_weight(node).cloned())
                .collect();
            findings.push(cycle_finding(&files));
        }
        for path in self_loops {
            findings.push(cycle_finding(&[path]));
        }
        findings
    }
}

fn cycle_finding(files: &[String]) -> Finding {
    let title = if files.len() == 1 {
        format!("Import cycle: {} imports itself", file_name(&files[0]))
    } else {
        format!("Import cycle across {} files", files.len())
    };
    let description = format!(
        "These modules import each other: {}. Cycles make initialization order fragile and hide unused code.",
        files.join(" → ")
    );
    let locations = files
        .iter()
        .map(|path| {
            Location::new(path.clone(), 1, 1).with_context("Part of an import cycle".to_string())
        })
        .collect();
    Finding::new(
        PatternType::ImportCycle,
        if files.len() > 3 {
            Severity::High
        } else {
            Severity::Medium
        },
        0.9,
        title,
        description,
    )
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
    .with_related_concepts(files.to_vec())
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
