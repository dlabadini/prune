//! Interface Sprawl detection - multiple inline types representing the same concept

use super::{Finding, Location, PatternDetector, PatternType, Severity};
use crate::config::Config;
use crate::db::{Database, Symbol, SymbolKind};
use similar::TextDiff;

pub struct InterfaceSprawlDetector;

impl PatternDetector for InterfaceSprawlDetector {
    fn pattern_type(&self) -> PatternType {
        PatternType::InterfaceSprawl
    }

    fn detect(&self, db: &Database, config: &Config) -> Vec<Finding> {
        let mut findings = Vec::new();

        let similarity_threshold = config.effective_interface_sprawl_similarity();
        let min_cluster_size = config.effective_interface_sprawl_min_cluster();

        // Get all interface and type alias definitions
        let mut type_symbols = Vec::new();

        if let Ok(interfaces) = db.get_symbols_by_kind(SymbolKind::Interface) {
            type_symbols.extend(interfaces);
        }
        if let Ok(type_aliases) = db.get_symbols_by_kind(SymbolKind::TypeAlias) {
            type_symbols.extend(type_aliases);
        }

        if type_symbols.len() < min_cluster_size {
            return findings;
        }

        // Find similar types using structural comparison
        let clusters = find_similar_clusters(&type_symbols, similarity_threshold, db);

        for cluster in clusters {
            if cluster.len() < min_cluster_size {
                continue;
            }

            // Check if types are across different files (more significant)
            let unique_files: std::collections::HashSet<_> = cluster.iter()
                .map(|s| s.file_id)
                .collect();

            if unique_files.len() < 2 {
                // All in same file - less concerning
                continue;
            }

            let severity = calculate_severity(cluster.len(), unique_files.len());
            let confidence = calculate_confidence(&cluster, similarity_threshold);

            let type_names: Vec<_> = cluster.iter().map(|s| s.name.clone()).collect();
            let concept = infer_concept_name(&type_names);

            let title = format!(
                "{} types represent '{}' concept",
                cluster.len(),
                concept
            );

            let description = format!(
                "Multiple type definitions appear to represent the same domain concept: {}. \
                These are spread across {} files.",
                type_names.join(", "),
                unique_files.len()
            );

            let rationale = String::from(
                "When the same concept is defined multiple times with slight variations:\n\
                - Changes require updates in multiple places\n\
                - Inconsistencies emerge over time (one type gets a new field, others don't)\n\
                - Developers waste time understanding which type to use\n\
                - Type safety is weakened by implicit conversions between similar types"
            );

            let mut locations = Vec::new();
            for symbol in &cluster {
                if let Ok(Some(path)) = db.get_file_path(symbol.file_id) {
                    let location = Location::new(
                        path,
                        symbol.start_line,
                        symbol.end_line,
                    )
                    .with_snippet(symbol.source_text.clone())
                    .with_context(format!("{}: {}", symbol.kind.as_str(), symbol.name));
                    locations.push(location);
                }
            }

            let suggested_fix = generate_fix_suggestion(&concept, &type_names, &locations);

            let finding = Finding::new(
                PatternType::InterfaceSprawl,
                severity,
                confidence,
                title,
                description,
            )
            .with_rationale(rationale)
            .with_locations(locations)
            .with_suggested_fix(suggested_fix)
            .with_related_concepts(type_names);

            findings.push(finding);
        }

        findings
    }
}

fn find_similar_clusters(
    symbols: &[Symbol],
    threshold: f32,
    _db: &Database,
) -> Vec<Vec<Symbol>> {
    let mut clusters: Vec<Vec<Symbol>> = Vec::new();
    let mut used: std::collections::HashSet<i64> = std::collections::HashSet::new();

    for (i, symbol) in symbols.iter().enumerate() {
        if used.contains(&symbol.id) {
            continue;
        }

        let mut cluster = vec![symbol.clone()];
        used.insert(symbol.id);

        for other in symbols.iter().skip(i + 1) {
            if used.contains(&other.id) {
                continue;
            }

            let similarity = calculate_type_similarity(symbol, other);
            if similarity >= threshold {
                cluster.push(other.clone());
                used.insert(other.id);
            }
        }

        if cluster.len() > 1 {
            clusters.push(cluster);
        }
    }

    clusters
}

fn calculate_type_similarity(a: &Symbol, b: &Symbol) -> f32 {
    // Normalize the source text for comparison
    let a_normalized = normalize_type_definition(&a.source_text);
    let b_normalized = normalize_type_definition(&b.source_text);

    // Calculate structural similarity
    let diff = TextDiff::from_lines(&a_normalized, &b_normalized);
    let ratio = diff.ratio();

    // Also consider name similarity
    let name_similarity = calculate_name_similarity(&a.name, &b.name);

    // Weight: 70% structure, 30% name
    ratio * 0.7 + name_similarity * 0.3
}

fn normalize_type_definition(source: &str) -> String {
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

fn calculate_name_similarity(a: &str, b: &str) -> f32 {
    // Extract base name (remove common suffixes like Props, Data, Info, Type, etc.)
    let a_base = extract_base_name(a);
    let b_base = extract_base_name(b);

    if a_base == b_base {
        1.0
    } else if a_base.contains(&b_base) || b_base.contains(&a_base) {
        0.8
    } else {
        // Use character-level similarity
        let diff = TextDiff::from_chars(&a_base, &b_base);
        diff.ratio()
    }
}

fn extract_base_name(name: &str) -> String {
    let suffixes = ["Props", "Data", "Info", "Type", "Interface", "DTO", "Model", "Entity", "Response", "Request"];

    let mut base = name.to_string();
    for suffix in suffixes {
        if let Some(stripped) = base.strip_suffix(suffix) {
            base = stripped.to_string();
            break;
        }
    }

    base.to_lowercase()
}

fn infer_concept_name(type_names: &[String]) -> String {
    // Try to find common base name
    let base_names: Vec<_> = type_names.iter()
        .map(|n| extract_base_name(n))
        .collect();

    // Find the most common base name
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for name in &base_names {
        *counts.entry(name.clone()).or_insert(0) += 1;
    }

    counts.into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(name, _)| {
            // Capitalize first letter
            let mut chars = name.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
                None => name,
            }
        })
        .unwrap_or_else(|| "Concept".to_string())
}

fn calculate_severity(cluster_size: usize, file_count: usize) -> Severity {
    let score = cluster_size * file_count;

    if score > 12 {
        Severity::High
    } else if score > 6 {
        Severity::Medium
    } else {
        Severity::Low
    }
}

fn calculate_confidence(cluster: &[Symbol], threshold: f32) -> f32 {
    // Higher similarity = higher confidence
    // More types = higher confidence this is a real pattern
    let size_factor = (cluster.len() as f32 / 3.0).min(1.0);
    let base_confidence = threshold;

    (base_confidence * 0.7 + size_factor * 0.3).min(0.95)
}

fn generate_fix_suggestion(concept: &str, type_names: &[String], locations: &[Location]) -> String {
    let type_files: Vec<String> = locations.iter()
        .map(|l| extract_filename(&l.file).to_string())
        .collect();
    let files_list = type_files.join("\n   - ");
    let types_list = type_names.join(", ");

    format!(
        r#"1. **Create canonical type** in `src/types/{concept_lower}.ts`:
   ```typescript
   export interface {concept} {{{{
     // Consolidate fields from all variants
     // Include union of all used fields
   }}}}
   ```

2. **Audit all variants** to identify:
   - Common fields (required in canonical type)
   - Variant-specific fields (consider optional or separate types)
   - Naming inconsistencies (e.g., `email` vs `emailAddress`)

3. **Replace inline types** in each file:
   - {files}

4. **For component props**, extend or pick from canonical type:
   ```typescript
   type {concept}Props = Pick<{concept}, 'id' | 'name'> & {{{{
     onClick: () => void;
   }}}};
   ```

5. **Remove unused type definitions** after migration

Types to consolidate: {types}"#,
        concept_lower = concept.to_lowercase(),
        concept = concept,
        files = files_list,
        types = types_list
    )
}

fn extract_filename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_base_name() {
        assert_eq!(extract_base_name("UserProps"), "user");
        assert_eq!(extract_base_name("UserData"), "user");
        assert_eq!(extract_base_name("User"), "user");
        assert_eq!(extract_base_name("UserResponseDTO"), "userresponse");
    }

    #[test]
    fn test_infer_concept_name() {
        let names = vec![
            "User".to_string(),
            "UserProps".to_string(),
            "UserData".to_string(),
        ];
        assert_eq!(infer_concept_name(&names), "User");
    }
}
