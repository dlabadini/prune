//! Structural overlap and composable bundle detection.

use super::type_shape::{
    bundle_signature, infer_bundle_name, parse_type_definition, top_level_bindings, FieldBinding,
    NestedSlot, ParsedTypeDef,
};
use crate::db::Symbol;
use crate::patterns::{candidate_pairs, simhash};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SprawlKind {
    Duplicate,
    PartialOverlap,
    NestedSlot,
    ComposableBundle,
    HierarchicalBundle,
}

#[derive(Debug, Clone)]
pub struct ChildBundle {
    pub name: String,
    pub fields: Vec<FieldBinding>,
}

#[derive(Debug, Clone)]
pub struct SprawlCluster {
    pub kind: SprawlKind,
    pub members: Vec<IndexedType>,
    pub shared_fields: Vec<FieldBinding>,
    pub child_bundles: Vec<ChildBundle>,
    pub nested_slot: Option<NestedSlot>,
    pub concept: String,
    pub confidence: f32,
}

#[derive(Debug, Clone)]
pub struct IndexedType {
    pub symbol: Symbol,
    pub parsed: ParsedTypeDef,
    pub bindings: Vec<FieldBinding>,
}

pub struct DetectionConfig {
    pub similarity_threshold: f32,
    pub min_cluster_size: usize,
    pub min_bundle_fields: usize,
    pub min_overlap_ratio: f32,
}

fn symbol_to_indexed(symbol: &Symbol) -> Option<IndexedType> {
    let parsed = parse_type_definition(&symbol.source_text, &symbol.name, symbol.kind)?;
    let bindings = top_level_bindings(&parsed);
    Some(IndexedType {
        symbol: symbol.clone(),
        parsed,
        bindings,
    })
}

pub fn index_types(symbols: &[Symbol]) -> Vec<IndexedType> {
    symbols
        .iter()
        .filter_map(|symbol| {
            let indexed = symbol_to_indexed(symbol)?;
            if indexed.parsed.is_boilerplate
                || !indexed.parsed.is_object_like
                || indexed.parsed.fields.len() < 2
                || is_noise_shape(&indexed.parsed)
            {
                return None;
            }
            Some(indexed)
        })
        .collect()
}

pub fn detect_legacy_text_clusters(
    symbols: &[Symbol],
    config: &DetectionConfig,
) -> Vec<SprawlCluster> {
    use similar::TextDiff;

    let mut clusters = Vec::new();
    let mut used = HashSet::new();
    for (left_index, left) in symbols.iter().enumerate() {
        if used.contains(&left.id) {
            continue;
        }
        let left_normalized = normalize_type_text(&left.source_text);
        let mut members = vec![left.clone()];
        for right in symbols.iter().skip(left_index + 1) {
            if used.contains(&right.id) || left.file_id == right.file_id {
                continue;
            }
            let right_normalized = normalize_type_text(&right.source_text);
            let ratio = TextDiff::from_lines(&left_normalized, &right_normalized).ratio();
            let name_similarity = name_similarity(&left.name, &right.name);
            let similarity = ratio * 0.7 + name_similarity * 0.3;
            if similarity >= config.similarity_threshold {
                members.push(right.clone());
            }
        }
        if members.len() >= config.min_cluster_size {
            for member in &members {
                used.insert(member.id);
            }
            let concept = infer_concept_from_names(
                &members.iter().map(|member| member.name.clone()).collect::<Vec<_>>(),
            );
            let indexed_members = members
                .into_iter()
                .filter_map(|symbol| symbol_to_indexed(&symbol))
                .collect::<Vec<_>>();
            if indexed_members.len() < config.min_cluster_size {
                continue;
            }
            clusters.push(SprawlCluster {
                kind: SprawlKind::Duplicate,
                members: indexed_members,
                shared_fields: Vec::new(),
                child_bundles: Vec::new(),
                nested_slot: None,
                concept,
                confidence: 0.75,
            });
        }
    }
    clusters
        .into_iter()
        .filter(|cluster| cluster.members.len() >= config.min_cluster_size)
        .collect()
}

fn normalize_type_text(source: &str) -> String {
    source
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
        .replace("interface ", "type ")
        .replace(";", "")
        .to_lowercase()
}

pub fn detect_sprawl_clusters(
    types: &[IndexedType],
    config: &DetectionConfig,
) -> Vec<SprawlCluster> {
    let mut clusters = Vec::new();
    clusters.extend(find_duplicate_clusters(types, config));
    if types.len() > 80 {
        clusters.extend(find_partial_overlap_banded(types, config));
    } else {
        clusters.extend(find_partial_overlap_clusters(types, config));
    }
    clusters.extend(find_nested_slot_clusters(types, config));
    clusters.extend(find_composable_bundle_clusters(types, config));
    clusters.extend(find_hierarchical_bundle_clusters(types, config));
    enrich_with_child_bundles(&mut clusters);
    dedupe_clusters(clusters)
}

/// Group shared fields into prefix-based child bundles (race*, entrant*, etc.).
pub fn mine_child_bundles(fields: &[FieldBinding]) -> Vec<ChildBundle> {
    if fields.len() < 4 {
        return Vec::new();
    }
    let mut groups: HashMap<String, Vec<FieldBinding>> = HashMap::new();
    for field in fields {
        let prefix = field_prefix(&field.name);
        if prefix.len() < 2 {
            continue;
        }
        groups.entry(prefix).or_default().push(field.clone());
    }
    let mut bundles: Vec<ChildBundle> = groups
        .into_iter()
        .filter(|(_, members)| members.len() >= 2)
        .map(|(prefix, members)| ChildBundle {
            name: format!("{}Facts", capitalize_token(&prefix)),
            fields: members,
        })
        .collect();
    bundles.sort_by_key(|bundle| bundle.name.clone());
    if bundles.len() >= 2 {
        bundles
    } else {
        Vec::new()
    }
}

fn enrich_with_child_bundles(clusters: &mut [SprawlCluster]) {
    for cluster in clusters.iter_mut() {
        if cluster.child_bundles.is_empty() {
            cluster.child_bundles = mine_child_bundles(&cluster.shared_fields);
        }
        if cluster.child_bundles.len() >= 2
            && matches!(
                cluster.kind,
                SprawlKind::PartialOverlap | SprawlKind::ComposableBundle
            )
        {
            cluster.kind = SprawlKind::HierarchicalBundle;
            cluster.confidence = cluster.confidence.max(0.9);
        }
    }
}

fn find_hierarchical_bundle_clusters(
    types: &[IndexedType],
    config: &DetectionConfig,
) -> Vec<SprawlCluster> {
    if types.len() > 120 {
        return Vec::new();
    }
    let mut clusters = Vec::new();
    for left in 0..types.len() {
        for right in (left + 1)..types.len() {
            let shared = matching_bindings(&types[left].bindings, &types[right].bindings);
            if shared.len() < config.min_bundle_fields + 2 {
                continue;
            }
            let child_bundles = mine_child_bundles(&shared);
            if child_bundles.len() < 2 {
                continue;
            }
            let left_extras = types[left].bindings.len() - shared.len();
            let right_extras = types[right].bindings.len() - shared.len();
            if left_extras == 0 && right_extras == 0 {
                continue;
            }
            let members = vec![types[left].clone(), types[right].clone()];
            let unique_files: HashSet<i64> = members.iter().map(|m| m.symbol.file_id).collect();
            if unique_files.len() < 2 {
                continue;
            }
            let concept = infer_bundle_name(&shared);
            clusters.push(SprawlCluster {
                kind: SprawlKind::HierarchicalBundle,
                members,
                shared_fields: shared,
                child_bundles,
                nested_slot: None,
                concept,
                confidence: 0.91,
            });
        }
    }
    clusters
}

fn field_prefix(name: &str) -> String {
    let lower = name.to_lowercase();
    for suffix in ["id", "name", "slug", "email", "token", "url", "count", "status"] {
        if let Some(stripped) = lower.strip_suffix(suffix) {
            if !stripped.is_empty() {
                return stripped.to_string();
            }
        }
    }
    if let Some(index) = name.find(|c: char| c.is_uppercase()) {
        return name[..index].to_lowercase();
    }
    name.to_lowercase()
}

fn capitalize_token(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => value.to_string(),
    }
}

fn find_duplicate_clusters(
    types: &[IndexedType],
    config: &DetectionConfig,
) -> Vec<SprawlCluster> {
    let mut clusters = Vec::new();
    let mut used = HashSet::new();

    for (left_index, left) in types.iter().enumerate() {
        if used.contains(&left.symbol.id) {
            continue;
        }
        let mut cluster = vec![left_index];
        for (right_index, right) in types.iter().enumerate().skip(left_index + 1) {
            if used.contains(&right.symbol.id) {
                continue;
            }
            let similarity = type_similarity(left, right);
            if similarity >= config.similarity_threshold {
                cluster.push(right_index);
            }
        }
        if cluster.len() >= config.min_cluster_size {
            for index in &cluster {
                used.insert(types[*index].symbol.id);
            }
            let members = cluster
                .iter()
                .map(|index| types[*index].clone())
                .collect::<Vec<_>>();
            let shared_fields = shared_bindings(&members);
            let concept = infer_concept_from_members(&members);
            clusters.push(SprawlCluster {
                kind: SprawlKind::Duplicate,
                members,
                shared_fields,
                child_bundles: Vec::new(),
                nested_slot: None,
                concept,
                confidence: 0.9,
            });
        }
    }
    clusters
}

fn find_partial_overlap_banded(
    types: &[IndexedType],
    config: &DetectionConfig,
) -> Vec<SprawlCluster> {
    let hashes: Vec<u64> = types
        .iter()
        .map(|ty| simhash(&bundle_signature(&ty.bindings)))
        .collect();
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); types.len()];
    for (left, right) in candidate_pairs(&hashes) {
        let shared = matching_bindings(&types[left].bindings, &types[right].bindings);
        if shared_overlap_ok(&types[left], &types[right], &shared, config) {
            adjacency[left].push(right);
            adjacency[right].push(left);
        }
    }
    collect_overlap_clusters(types, config, adjacency)
}

fn find_partial_overlap_clusters(
    types: &[IndexedType],
    config: &DetectionConfig,
) -> Vec<SprawlCluster> {
    let mut adjacency: Vec<Vec<usize>> = vec![Vec::new(); types.len()];
    for left in 0..types.len() {
        for right in (left + 1)..types.len() {
            let shared = matching_bindings(&types[left].bindings, &types[right].bindings);
            if shared_overlap_ok(&types[left], &types[right], &shared, config) {
                adjacency[left].push(right);
                adjacency[right].push(left);
            }
        }
    }
    collect_overlap_clusters(types, config, adjacency)
}

fn shared_overlap_ok(
    left: &IndexedType,
    right: &IndexedType,
    shared: &[FieldBinding],
    config: &DetectionConfig,
) -> bool {
    if shared.len() < config.min_bundle_fields {
        return false;
    }
    let overlap_ratio =
        shared.len() as f32 / left.bindings.len().min(right.bindings.len()) as f32;
    let has_unique_fields =
        shared.len() < left.bindings.len() || shared.len() < right.bindings.len();
    has_unique_fields && overlap_ratio >= config.min_overlap_ratio
}

fn collect_overlap_clusters(
    types: &[IndexedType],
    config: &DetectionConfig,
    adjacency: Vec<Vec<usize>>,
) -> Vec<SprawlCluster> {
    let mut visited = vec![false; types.len()];
    let mut clusters = Vec::new();
    for start in 0..types.len() {
        if visited[start] || adjacency[start].is_empty() {
            continue;
        }
        let component = collect_component(start, &adjacency, &mut visited);
        if component.len() < config.min_cluster_size {
            continue;
        }
        let members = component
            .iter()
            .map(|index| types[*index].clone())
            .collect::<Vec<_>>();
        let shared_fields = intersection_bindings(&members);
        if shared_fields.len() < config.min_bundle_fields {
            continue;
        }
        let concept = infer_bundle_name(&shared_fields);
        clusters.push(SprawlCluster {
            kind: SprawlKind::PartialOverlap,
            members,
            shared_fields,
            child_bundles: Vec::new(),
            nested_slot: None,
            concept,
            confidence: 0.82,
        });
    }
    clusters
}

fn find_nested_slot_clusters(
    types: &[IndexedType],
    config: &DetectionConfig,
) -> Vec<SprawlCluster> {
    let mut groups: HashMap<String, Vec<(usize, NestedSlot)>> = HashMap::new();
    for (index, ty) in types.iter().enumerate() {
        for binding in &ty.bindings {
            if let Some(slot) = binding.nested.clone() {
                if slot.fields.len() < 2 {
                    continue;
                }
                groups
                    .entry(slot.signature.clone())
                    .or_default()
                    .push((index, slot));
            }
        }
    }

    let mut clusters = Vec::new();
    for (_signature, entries) in groups {
        let unique_types: HashSet<usize> = entries.iter().map(|(index, _)| *index).collect();
        if unique_types.len() < config.min_cluster_size {
            continue;
        }
        let unique_files: HashSet<i64> = unique_types
            .iter()
            .map(|index| types[*index].symbol.file_id)
            .collect();
        if unique_files.len() < 2 {
            continue;
        }
        let members = unique_types
            .iter()
            .map(|index| types[*index].clone())
            .collect::<Vec<_>>();
        let nested_slot = entries.first().map(|(_, slot)| slot.clone());
        let shared_fields = nested_slot
            .as_ref()
            .map(|slot| slot.fields.clone())
            .unwrap_or_default();
        let concept = nested_slot
            .as_ref()
            .map(|slot| infer_bundle_name(&slot.fields))
            .unwrap_or_else(|| "NestedSlot".to_string());
        clusters.push(SprawlCluster {
            kind: SprawlKind::NestedSlot,
            members,
            shared_fields,
            child_bundles: Vec::new(),
            nested_slot,
            concept,
            confidence: 0.86,
        });
    }
    clusters
}

fn find_composable_bundle_clusters(
    types: &[IndexedType],
    config: &DetectionConfig,
) -> Vec<SprawlCluster> {
    let mut bundle_counts: HashMap<String, (Vec<FieldBinding>, HashSet<usize>)> = HashMap::new();

    for (index, ty) in types.iter().enumerate() {
        let binding_count = ty.bindings.len();
        if binding_count < config.min_bundle_fields || binding_count > 10 {
            continue;
        }
        let size = config.min_bundle_fields;
        if binding_count < size {
            continue;
        }
        for subset in combinations(&ty.bindings, size) {
            let signature = bundle_signature(&subset);
            let entry = bundle_counts
                .entry(signature)
                .or_insert_with(|| (subset.clone(), HashSet::new()));
            entry.1.insert(index);
        }
    }

    let mut clusters = Vec::new();
    for (bindings, member_indexes) in bundle_counts.values() {
        if member_indexes.len() < config.min_cluster_size {
            continue;
        }
        let members = member_indexes
            .iter()
            .map(|index| types[*index].clone())
            .collect::<Vec<_>>();
        let unique_files: HashSet<i64> = members.iter().map(|member| member.symbol.file_id).collect();
        if unique_files.len() < 2 {
            continue;
        }
        let all_exact = members
            .iter()
            .all(|member| matching_bindings(bindings, &member.bindings).len() == bindings.len());
        if !all_exact {
            continue;
        }
        let has_variants = members.iter().any(|member| member.bindings.len() > bindings.len());
        if !has_variants {
            continue;
        }
        let concept = infer_bundle_name(bindings);
        clusters.push(SprawlCluster {
            kind: SprawlKind::ComposableBundle,
            members,
            shared_fields: bindings.clone(),
            child_bundles: Vec::new(),
            nested_slot: None,
            concept,
            confidence: 0.88,
        });
    }
    clusters
}

fn type_similarity(left: &IndexedType, right: &IndexedType) -> f32 {
    let shared = matching_bindings(&left.bindings, &right.bindings);
    if left.bindings.is_empty() || right.bindings.is_empty() {
        return 0.0;
    }
    let union = left.bindings.len() + right.bindings.len() - shared.len();
    let jaccard = shared.len() as f32 / union as f32;
    let coverage = shared.len() as f32
        / left.bindings.len().max(right.bindings.len()) as f32;
    let name_similarity = name_similarity(&left.parsed.name, &right.parsed.name);
    jaccard * 0.55 + coverage * 0.25 + name_similarity * 0.2
}

fn matching_bindings(left: &[FieldBinding], right: &[FieldBinding]) -> Vec<FieldBinding> {
    let right_map: HashMap<&str, &FieldBinding> = right
        .iter()
        .map(|binding| (binding.name.as_str(), binding))
        .collect();
    left.iter()
        .filter_map(|binding| {
            right_map
                .get(binding.name.as_str())
                .filter(|other| binding.signature == other.signature)
                .map(|_| binding.clone())
        })
        .collect()
}

fn shared_bindings(members: &[IndexedType]) -> Vec<FieldBinding> {
    if members.is_empty() {
        return Vec::new();
    }
    intersection_bindings(members)
}

fn intersection_bindings(members: &[IndexedType]) -> Vec<FieldBinding> {
    let mut shared = members[0].bindings.clone();
    for member in members.iter().skip(1) {
        shared = matching_bindings(&shared, &member.bindings);
    }
    shared
}

fn collect_component(
    start: usize,
    adjacency: &[Vec<usize>],
    visited: &mut [bool],
) -> Vec<usize> {
    let mut stack = vec![start];
    let mut component = Vec::new();
    visited[start] = true;
    while let Some(node) = stack.pop() {
        component.push(node);
        for neighbor in &adjacency[node] {
            if !visited[*neighbor] {
                visited[*neighbor] = true;
                stack.push(*neighbor);
            }
        }
    }
    component
}

fn combinations(items: &[FieldBinding], size: usize) -> Vec<Vec<FieldBinding>> {
    if size == 0 || items.len() < size {
        return Vec::new();
    }
    let mut current = Vec::new();
    let mut out = Vec::new();
    combine_recursive(items, size, 0, &mut current, &mut out);
    out
}

fn combine_recursive(
    items: &[FieldBinding],
    size: usize,
    start: usize,
    current: &mut Vec<FieldBinding>,
    out: &mut Vec<Vec<FieldBinding>>,
) {
    if current.len() == size {
        out.push(current.clone());
        return;
    }
    let remaining = size - current.len();
    for index in start..=items.len().saturating_sub(remaining) {
        current.push(items[index].clone());
        combine_recursive(items, size, index + 1, current, out);
        current.pop();
    }
}

fn is_noise_shape(parsed: &ParsedTypeDef) -> bool {
    if parsed.name.ends_with("Ctx") || parsed.name.ends_with("Context") {
        return true;
    }
    if parsed.fields.len() == 1 && parsed.fields[0].name == "db" {
        return true;
    }
    false
}

fn dedupe_clusters(clusters: Vec<SprawlCluster>) -> Vec<SprawlCluster> {
    let mut unique = Vec::new();
    for cluster in clusters {
        let member_ids: Vec<i64> = cluster
            .members
            .iter()
            .map(|member| member.symbol.id)
            .collect();
        let duplicate = unique.iter().any(|existing: &SprawlCluster| {
            let existing_ids: HashSet<i64> = existing
                .members
                .iter()
                .map(|member| member.symbol.id)
                .collect();
            let overlap = member_ids
                .iter()
                .filter(|id| existing_ids.contains(id))
                .count();
            overlap >= cluster.members.len().min(existing.members.len())
        });
        if !duplicate {
            unique.push(cluster);
        }
    }
    unique
}

fn infer_concept_from_members(members: &[IndexedType]) -> String {
    let names = members
        .iter()
        .map(|member| member.parsed.name.clone())
        .collect::<Vec<_>>();
    infer_concept_from_names(&names)
}

pub fn infer_concept_from_names(type_names: &[String]) -> String {
    let base_names: Vec<_> = type_names.iter().map(|name| extract_base_name(name)).collect();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for name in &base_names {
        *counts.entry(name.clone()).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(name, _)| capitalize(&name))
        .unwrap_or_else(|| "Shared".to_string())
}

pub fn extract_base_name(name: &str) -> String {
    let suffixes = [
        "Props",
        "Data",
        "Info",
        "Type",
        "Interface",
        "DTO",
        "Model",
        "Entity",
        "Response",
        "Request",
        "Args",
        "Row",
        "State",
        "Wire",
        "Facts",
        "Config",
    ];
    let mut base = name.to_string();
    for suffix in suffixes {
        if let Some(stripped) = base.strip_suffix(suffix) {
            base = stripped.to_string();
            break;
        }
    }
    base.to_lowercase()
}

pub fn is_generic_concept(name: &str) -> bool {
    name.len() <= 2
        || matches!(
            name,
            "Props"
                | "Type"
                | "State"
                | "Config"
                | "Options"
                | "Context"
                | "Concept"
                | "Shared"
                | "Nestedslot"
                | "Sharedfacts"
        )
}

fn name_similarity(left: &str, right: &str) -> f32 {
    let left_base = extract_base_name(left);
    let right_base = extract_base_name(right);
    if left_base == right_base {
        1.0
    } else if left_base.contains(&right_base) || right_base.contains(&left_base) {
        0.8
    } else {
        0.3
    }
}

fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Symbol, SymbolKind};

    fn indexed(source: &str, name: &str, id: i64, file_id: i64) -> IndexedType {
        let parsed = parse_type_definition(source, name, SymbolKind::Interface).unwrap();
        let bindings = top_level_bindings(&parsed);
        IndexedType {
            symbol: Symbol {
                id,
                file_id,
                name: name.to_string(),
                kind: SymbolKind::Interface,
                start_line: 1,
                end_line: 10,
                is_exported: true,
                is_default: false,
                source_text: source.to_string(),
                callee: None,
            },
            parsed,
            bindings,
        }
    }

    #[test]
    fn finds_partial_overlap_cluster() {
        let a = indexed(
            "interface TransferClaimNotifyArgs { raceId: string; raceName: string; raceSlug: string; claimToken: string; claimExpiresAt: number; recipientEmail: string; }",
            "TransferClaimNotifyArgs",
            1,
            10,
        );
        let b = indexed(
            "interface TransferReminderNotifyArgs { raceId: string; raceName: string; raceSlug: string; recipientEmail: string; daysRemaining: number; }",
            "TransferReminderNotifyArgs",
            2,
            11,
        );
        let c = indexed(
            "interface TransferReviewRow { raceSlug: string; raceName: string; recipientEmail: string; raceId: string; entrantName: string; status: string; }",
            "TransferReviewRow",
            3,
            12,
        );
        let config = DetectionConfig {
            similarity_threshold: 0.85,
            min_cluster_size: 3,
            min_bundle_fields: 3,
            min_overlap_ratio: 0.45,
        };
        let clusters = detect_sprawl_clusters(&[a, b, c], &config);
        assert!(
            clusters.iter().any(|cluster| {
                cluster.kind == SprawlKind::PartialOverlap
                    || cluster.kind == SprawlKind::ComposableBundle
            })
        );
    }

    #[test]
    fn finds_nested_slot_cluster() {
        let a = indexed(
            "interface A { race: { name: string; slug: string }; extra: string; }",
            "A",
            1,
            1,
        );
        let b = indexed(
            "interface B { event: { name: string; slug: string }; other: number; }",
            "B",
            2,
            2,
        );
        let c = indexed(
            "interface C { target: { slug: string; name: string }; flag: boolean; }",
            "C",
            3,
            3,
        );
        let config = DetectionConfig {
            similarity_threshold: 0.85,
            min_cluster_size: 3,
            min_bundle_fields: 2,
            min_overlap_ratio: 0.45,
        };
        let clusters = detect_sprawl_clusters(&[a, b, c], &config);
        assert!(clusters.iter().any(|cluster| cluster.kind == SprawlKind::NestedSlot));
    }
}
