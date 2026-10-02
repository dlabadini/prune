//! Interface Sprawl detection - duplicate, partial-overlap, and composable bundles.

pub mod detect;
pub mod type_shape;

pub use detect::{extract_base_name, index_types, IndexedType};

use detect::{
    detect_legacy_text_clusters, detect_sprawl_clusters, is_generic_concept, DetectionConfig,
    SprawlCluster, SprawlKind,
};
use type_shape::infer_bundle_name;

use super::{Finding, Location, PatternDetector, PatternType, Severity};
use crate::compose::{
    compose_from_sprawl, format_action_markdown, format_action_summary, ChildBundleInput,
    MemberInput, SprawlComposeInput,
};
use crate::config::Config;
use crate::db::{Database, Symbol, SymbolKind};

pub struct InterfaceSprawlDetector;

impl PatternDetector for InterfaceSprawlDetector {
    fn pattern_type(&self) -> PatternType {
        PatternType::InterfaceSprawl
    }

    fn detect(&self, db: &Database, config: &Config) -> Vec<Finding> {
        let type_symbols = shape_symbols(db);

        let min_cluster_size = config.effective_interface_sprawl_min_cluster();
        if type_symbols.len() < min_cluster_size {
            return Vec::new();
        }

        let indexed = detect::index_types(&type_symbols);
        if indexed.len() < min_cluster_size {
            return Vec::new();
        }

        let detection_config = DetectionConfig {
            similarity_threshold: config.effective_interface_sprawl_similarity(),
            min_cluster_size,
            min_bundle_fields: config.effective_interface_sprawl_min_bundle_fields(),
            min_overlap_ratio: config.effective_interface_sprawl_min_overlap_ratio(),
        };

        let indexed_ids: std::collections::HashSet<i64> =
            indexed.iter().map(|ty| ty.symbol.id).collect();
        let unindexed = type_symbols
            .iter()
            .filter(|symbol| !indexed_ids.contains(&symbol.id))
            .cloned()
            .collect::<Vec<_>>();

        let mut clusters = detect_sprawl_clusters(&indexed, &detection_config);
        clusters.extend(detect_legacy_text_clusters(&unindexed, &detection_config));
        let max_findings = config.effective_interface_sprawl_max_findings();

        let mut findings = split_by_boundaries(db, config, clusters)
            .into_iter()
            .filter_map(|(cluster, boundary)| {
                cluster_to_finding(db, &cluster).map(|finding| label_boundary(finding, boundary))
            })
            .collect::<Vec<_>>();

        findings.sort_by(|left, right| {
            right
                .severity
                .cmp(&left.severity)
                .then_with(|| {
                    right
                        .confidence
                        .partial_cmp(&left.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                // Deterministic tie-break so the cap keeps the same findings every run.
                .then_with(|| left.identity_keys().cmp(&right.identity_keys()))
        });
        findings.truncate(max_findings);
        findings
    }
}

/// Interfaces, object type aliases, and object validators (`v.object`, `z.object`).
pub fn shape_symbols(db: &Database) -> Vec<Symbol> {
    let mut symbols = Vec::new();
    if let Ok(interfaces) = db.get_symbols_by_kind(SymbolKind::Interface) {
        symbols.extend(interfaces);
    }
    if let Ok(type_aliases) = db.get_symbols_by_kind(SymbolKind::TypeAlias) {
        symbols.extend(type_aliases);
    }
    if let Ok(variables) = db.get_symbols_by_kind(SymbolKind::Variable) {
        symbols.extend(variables.into_iter().filter(|symbol| {
            symbol
                .callee
                .as_deref()
                .is_some_and(type_shape::is_object_validator_callee)
        }));
    }
    symbols
}

/// Clusters that may be reported, split at fenced boundaries, with their boundary name.
fn split_by_boundaries(
    db: &Database,
    config: &Config,
    clusters: Vec<SprawlCluster>,
) -> Vec<(SprawlCluster, Option<String>)> {
    if config.boundaries.is_empty() {
        return clusters.into_iter().map(|cluster| (cluster, None)).collect();
    }
    let mut out = Vec::new();
    for cluster in clusters {
        let paths: Vec<String> = cluster
            .members
            .iter()
            .map(|member| {
                db.get_file_path(member.symbol.file_id)
                    .ok()
                    .flatten()
                    .unwrap_or_default()
            })
            .collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        let groups = config.partition_by_boundary(&refs);
        if groups.len() == 1 {
            let (label, _) = groups.into_iter().next().unwrap_or((None, Vec::new()));
            out.push((cluster, label));
            continue;
        }
        for (label, indexes) in groups {
            if indexes.len() < 2 {
                continue;
            }
            let mut part = cluster.clone();
            part.members = indexes
                .iter()
                .map(|index| cluster.members[*index].clone())
                .collect();
            out.push((part, label));
        }
    }
    out
}

pub fn label_boundary(mut finding: Finding, boundary: Option<String>) -> Finding {
    if let Some(name) = boundary {
        finding.title = format!("[{name}] {}", finding.title);
        finding.related_concepts.push(format!("boundary:{name}"));
    }
    finding
}

fn cluster_to_finding(db: &Database, cluster: &SprawlCluster) -> Option<Finding> {
    let unique_files: std::collections::HashSet<_> = cluster
        .members
        .iter()
        .map(|member| member.symbol.file_id)
        .collect();
    if unique_files.len() < 2 {
        return None;
    }
    if is_generic_concept(&cluster.concept) {
        return None;
    }
    if cluster.shared_fields.len() < 2 && cluster.kind != SprawlKind::Duplicate {
        return None;
    }

    let type_names = cluster
        .members
        .iter()
        .map(|member| member.parsed.name.clone())
        .collect::<Vec<_>>();
    let severity = calculate_severity(cluster.members.len(), unique_files.len(), cluster.kind);
    let title = title_for_cluster(cluster);
    let description = description_for_cluster(cluster, unique_files.len());
    let rationale = rationale_for_cluster(cluster);
    let compose_input = sprawl_compose_input(db, cluster);
    let action = compose_from_sprawl(&compose_input);
    let action_summary = format_action_summary(&action);
    let suggested_fix = format!(
        "{}\n\n{}",
        generate_fix_suggestion(cluster),
        format_action_markdown(&action)
    );

    let mut locations = Vec::new();
    for member in &cluster.members {
        if let Ok(Some(path)) = db.get_file_path(member.symbol.file_id) {
            locations.push(
                Location::new(path, member.symbol.start_line, member.symbol.end_line)
                    .with_snippet(member.symbol.source_text.clone())
                    .with_context(format!(
                        "{}: {}",
                        member.symbol.kind.as_str(),
                        member.symbol.name
                    )),
            );
        }
    }

    Some(
        Finding::new(
            PatternType::InterfaceSprawl,
            severity,
            cluster.confidence,
            title,
            description,
        )
        .with_rationale(rationale)
        .with_locations(locations)
        .with_suggested_fix(suggested_fix)
        .with_related_concepts(type_names)
        .with_prune_action(action)
        .with_action_summary(action_summary),
    )
}

fn sprawl_compose_input(db: &Database, cluster: &SprawlCluster) -> SprawlComposeInput {
    let members = cluster
        .members
        .iter()
        .filter_map(|member| {
            db.get_file_path(member.symbol.file_id)
                .ok()
                .flatten()
                .map(|path| MemberInput {
                    type_name: member.parsed.name.clone(),
                    file: path,
                    line: member.symbol.start_line,
                    bindings: member.bindings.clone(),
                })
        })
        .collect();
    SprawlComposeInput {
        concept: cluster.concept.clone(),
        shared_fields: cluster.shared_fields.clone(),
        child_bundles: cluster
            .child_bundles
            .iter()
            .map(|child| ChildBundleInput {
                name: child.name.clone(),
                fields: child.fields.clone(),
            })
            .collect(),
        members,
    }
}

fn title_for_cluster(cluster: &SprawlCluster) -> String {
    match cluster.kind {
        SprawlKind::Duplicate => format!(
            "{} duplicate type definitions for '{}'",
            cluster.members.len(),
            cluster.concept
        ),
        SprawlKind::PartialOverlap => format!(
            "{} types share composable '{}' bundle",
            cluster.members.len(),
            cluster.concept
        ),
        SprawlKind::NestedSlot => format!(
            "{} types repeat nested '{}' object shape",
            cluster.members.len(),
            cluster.concept
        ),
        SprawlKind::ComposableBundle => format!(
            "{} types restate '{}' field group",
            cluster.members.len(),
            cluster.concept
        ),
        SprawlKind::HierarchicalBundle => format!(
            "{} types share hierarchical '{}' bundles",
            cluster.members.len(),
            cluster.concept
        ),
    }
}

fn description_for_cluster(cluster: &SprawlCluster, file_count: usize) -> String {
    let type_names = cluster
        .members
        .iter()
        .map(|member| member.parsed.name.clone())
        .collect::<Vec<_>>();
    let shared = cluster
        .shared_fields
        .iter()
        .map(|field| field.name.clone())
        .collect::<Vec<_>>()
        .join(", ");

    match cluster.kind {
        SprawlKind::Duplicate => format!(
            "Near-identical type definitions ({}) appear across {} files. Shared fields: {}.",
            type_names.join(", "),
            file_count,
            shared
        ),
        SprawlKind::PartialOverlap => format!(
            "Types partially overlap and repeat the same child groupings instead of composing them: {}. \
            Shared bundle ({} fields): {}. Spread across {} files.",
            type_names.join(", "),
            cluster.shared_fields.len(),
            shared,
            file_count
        ),
        SprawlKind::NestedSlot => format!(
            "The same nested object shape is hand-rolled under different parent fields in {} types: {}. \
            Nested fields: {}.",
            cluster.members.len(),
            type_names.join(", "),
            shared
        ),
        SprawlKind::ComposableBundle => format!(
            "A stable field bundle appears in {} types with different extras: {}. \
            Extract bundle: {}.",
            cluster.members.len(),
            type_names.join(", "),
            shared
        ),
        SprawlKind::HierarchicalBundle => {
            let child_names = cluster
                .child_bundles
                .iter()
                .map(|child| child.name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Types repeat multiple composable child groupings instead of hierarchy: {}. \
                Child bundles: {}. Shared fields: {}.",
                type_names.join(", "),
                child_names,
                shared
            )
        }
    }
}

fn rationale_for_cluster(cluster: &SprawlCluster) -> String {
    match cluster.kind {
        SprawlKind::Duplicate => String::from(
            "Duplicate type definitions drift apart over time and force parallel edits for the same domain concept.",
        ),
        SprawlKind::PartialOverlap
        | SprawlKind::ComposableBundle
        | SprawlKind::HierarchicalBundle => String::from(
            "When interfaces share child groupings but restate them inline:\n\
            - Partial overlaps hide the real domain bundles (race, entrant, pricing, etc.)\n\
            - Subset/superset variants diverge on the shared fields first\n\
            - Composition via `extends` / `Pick` / shared `*Facts` types keeps each layer additive",
        ),
        SprawlKind::NestedSlot => String::from(
            "Repeated inline nested objects are a strong signal that a named slot type should exist \
            (for example `RaceIdentity`, `PricedLineItemsFacts`) and be composed by parent interfaces.",
        ),
    }
}

fn calculate_severity(cluster_size: usize, file_count: usize, kind: SprawlKind) -> Severity {
    let weight = match kind {
        SprawlKind::Duplicate => 1,
        SprawlKind::PartialOverlap
        | SprawlKind::ComposableBundle
        | SprawlKind::HierarchicalBundle => 2,
        SprawlKind::NestedSlot => 2,
    };
    let score = cluster_size * file_count * weight;
    if score > 16 {
        Severity::High
    } else if score > 8 {
        Severity::Medium
    } else {
        Severity::Low
    }
}

fn generate_fix_suggestion(cluster: &SprawlCluster) -> String {
    let bundle_name = if cluster.concept.ends_with("Facts") {
        cluster.concept.clone()
    } else {
        infer_bundle_name(&cluster.shared_fields)
    };
    let shared_fields = cluster
        .shared_fields
        .iter()
        .map(|field| {
            let optional = if field.optional { "?" } else { "" };
            format!("  {}{}: /* {} */;", field.name, optional, field.signature)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let type_names = cluster
        .members
        .iter()
        .map(|member| member.parsed.name.clone())
        .collect::<Vec<_>>()
        .join(", ");

    let nested_note = if let Some(slot) = &cluster.nested_slot {
        format!(
            "\n\nNested slot currently appears as `{}` with shape `{}`.",
            slot.field_name, slot.signature
        )
    } else {
        String::new()
    };

    let compose_examples = cluster
        .members
        .iter()
        .take(3)
        .map(|member| {
            let extras = member
                .bindings
                .iter()
                .filter(|binding| {
                    !cluster
                        .shared_fields
                        .iter()
                        .any(|shared| shared.name == binding.name)
                })
                .map(|binding| binding.name.clone())
                .collect::<Vec<_>>();
            if extras.is_empty() {
                format!("interface {} extends {} {{}}", member.parsed.name, bundle_name)
            } else {
                format!(
                    "interface {} extends {} {{\n  // unique: {}\n}}",
                    member.parsed.name,
                    bundle_name,
                    extras.join(", ")
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "1. **Extract shared bundle** in a domain types module:\n\
```typescript\nexport interface {bundle_name} {{\n{shared_fields}\n}}\n```\n\n\
2. **Compose, don't flatten** — keep each consumer additive:\n\
```typescript\n{compose_examples}\n```\n\n\
3. **For nested slots**, name the child object once and reuse it under each parent field.\n\n\
Types involved: {type_names}{nested_note}",
        bundle_name = bundle_name,
        shared_fields = shared_fields,
        compose_examples = compose_examples,
        type_names = type_names,
        nested_note = nested_note
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use detect::{extract_base_name, infer_concept_from_names};
    use type_shape::shape_signature;

    #[test]
    fn test_extract_base_name() {
        assert_eq!(extract_base_name("UserProps"), "user");
        assert_eq!(extract_base_name("TransferClaimNotifyArgs"), "transferclaimnotify");
    }

    #[test]
    fn test_infer_concept_name() {
        let names = vec![
            "User".to_string(),
            "UserProps".to_string(),
            "UserData".to_string(),
        ];
        assert_eq!(infer_concept_from_names(&names), "User");
    }

    #[test]
    fn nested_shape_signature_is_order_insensitive() {
        let left = shape_signature(&type_shape::TypeShape::AnonymousObject(vec![
            type_shape::TypeField {
                name: "name".to_string(),
                optional: false,
                shape: type_shape::TypeShape::Primitive("string".to_string()),
            },
            type_shape::TypeField {
                name: "slug".to_string(),
                optional: false,
                shape: type_shape::TypeShape::Primitive("string".to_string()),
            },
        ]));
        let right = shape_signature(&type_shape::TypeShape::AnonymousObject(vec![
            type_shape::TypeField {
                name: "slug".to_string(),
                optional: false,
                shape: type_shape::TypeShape::Primitive("string".to_string()),
            },
            type_shape::TypeField {
                name: "name".to_string(),
                optional: false,
                shape: type_shape::TypeShape::Primitive("string".to_string()),
            },
        ]));
        assert_eq!(left, right);
    }
}
