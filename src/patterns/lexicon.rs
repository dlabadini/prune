//! Lexicon cleanliness — naming drift on shared shapes and resource smash in identifiers.

use super::{Finding, Location, PatternDetector, PatternType, Severity};
use crate::compose::{compose_from_lexicon, format_action_markdown, format_action_summary};
use crate::config::Config;
use crate::db::{Database, Symbol, SymbolKind};
use crate::patterns::interface_sprawl::{
    extract_base_name, index_types, type_shape::bundle_signature, IndexedType,
};
use std::collections::{HashMap, HashSet};

pub struct LexiconDetector;

impl PatternDetector for LexiconDetector {
    fn pattern_type(&self) -> PatternType {
        PatternType::Lexicon
    }

    fn detect(&self, db: &Database, config: &Config) -> Vec<Finding> {
        let mut symbols = Vec::new();
        if let Ok(interfaces) = db.get_symbols_by_kind(SymbolKind::Interface) {
            symbols.extend(interfaces);
        }
        if let Ok(aliases) = db.get_symbols_by_kind(SymbolKind::TypeAlias) {
            symbols.extend(aliases);
        }
        if symbols.is_empty() {
            return Vec::new();
        }

        let indexed = index_types(&symbols);
        let min_fields = config.effective_lexicon_min_fields();
        let max_smash = config.effective_lexicon_max_smash_score();
        let max_findings = config.effective_lexicon_max_findings();

        let mut findings = Vec::new();
        findings.extend(detect_homonym_clusters(db, &indexed, min_fields));
        findings.extend(detect_smash_names(db, &symbols, max_smash));

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
        });
        findings.truncate(max_findings);
        findings
    }
}

fn detect_homonym_clusters(
    db: &Database,
    types: &[IndexedType],
    min_fields: usize,
) -> Vec<Finding> {
    let mut shape_groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, ty) in types.iter().enumerate() {
        if ty.bindings.len() < min_fields {
            continue;
        }
        let signature = bundle_signature(&ty.bindings);
        shape_groups.entry(signature).or_default().push(index);
    }

    let mut findings = Vec::new();
    for (signature, indexes) in shape_groups {
        if indexes.len() < 2 {
            continue;
        }
        let names: Vec<String> = indexes
            .iter()
            .map(|index| types[*index].parsed.name.clone())
            .collect();
        let bases: HashSet<String> = names.iter().map(|name| extract_base_name(name)).collect();
        if bases.len() < 2 {
            continue;
        }
        let unique_files: HashSet<i64> = indexes
            .iter()
            .map(|index| types[*index].symbol.file_id)
            .collect();
        if unique_files.len() < 2 {
            continue;
        }

        let canonical = pick_canonical_name(&names);
        let homonyms: Vec<(String, String, usize)> = indexes
            .iter()
            .filter_map(|index| {
                db.get_file_path(types[*index].symbol.file_id)
                    .ok()
                    .flatten()
                    .map(|path| {
                        (
                            types[*index].parsed.name.clone(),
                            path,
                            types[*index].symbol.start_line,
                        )
                    })
            })
            .collect();
        if homonyms.len() < 2 {
            continue;
        }

        let homonym_refs: Vec<(&str, &str, usize)> = homonyms
            .iter()
            .map(|(name, file, line)| (name.as_str(), file.as_str(), *line))
            .collect();
        let action = compose_from_lexicon(&canonical, &homonym_refs, &signature);
        let action_summary = format_action_summary(&action);
        let suggested_fix = format_action_markdown(&action);
        let mut locations = Vec::new();
        for index in &indexes {
            let ty = &types[*index];
            if let Ok(Some(path)) = db.get_file_path(ty.symbol.file_id) {
                locations.push(
                    Location::new(path, ty.symbol.start_line, ty.symbol.end_line)
                        .with_snippet(ty.symbol.source_text.clone())
                        .with_context(format!("{}: {}", ty.symbol.kind.as_str(), ty.parsed.name)),
                );
            }
        }

        let title = format!(
            "{} names for the same {}-field shape",
            names.len(),
            min_fields.max(
                types[indexes[0]]
                    .bindings
                    .len()
            )
        );
        let description = format!(
            "Identical field bags use divergent concept names: {}. \
            Canonical suggestion: `{}`. Shape: {}.",
            names.join(", "),
            canonical,
            field_names_preview(&types[indexes[0]].bindings)
        );

        findings.push(
            Finding::new(
                PatternType::Lexicon,
                if names.len() > 3 {
                    Severity::High
                } else {
                    Severity::Medium
                },
                0.84,
                title,
                description,
            )
            .with_rationale(
                "When the same shape carries different names, developers cannot tell which type is canonical. \
                Lexicon drift is separate from interface sprawl: the fix is naming alignment, not merging fields."
                    .to_string(),
            )
            .with_locations(locations)
            .with_suggested_fix(suggested_fix)
            .with_related_concepts(names)
            .with_prune_action(action)
            .with_action_summary(action_summary),
        );
    }
    findings
}

fn detect_smash_names(db: &Database, symbols: &[Symbol], max_smash: usize) -> Vec<Finding> {
    let mut findings = Vec::new();
    for symbol in symbols {
        let score = smash_score(&symbol.name);
        if score <= max_smash {
            continue;
        }
        if symbol.name.ends_with("Ctx") || symbol.name.ends_with("Context") {
            continue;
        }
        let tokens = split_camel_tokens(&symbol.name);
        if let Ok(Some(path)) = db.get_file_path(symbol.file_id) {
            let action = compose_from_lexicon(
                &suggest_smashed_name(&tokens),
                &[(
                    symbol.name.as_str(),
                    path.as_str(),
                    symbol.start_line,
                )],
                "identifier",
            );
            let action_summary = format_action_summary(&action);
            let suggested_fix = format_action_markdown(&action);
            findings.push(
                Finding::new(
                    PatternType::Lexicon,
                    if score > max_smash + 1 {
                        Severity::Medium
                    } else {
                        Severity::Low
                    },
                    0.78,
                    format!("`{}` packs {} resource tokens", symbol.name, score),
                    format!(
                        "Identifier `{}` composes many domain resources in one name ({}). \
                        Prefer one resource atom per type and compose with `extends` or intersection.",
                        symbol.name,
                        tokens.join(" + ")
                    ),
                )
                .with_rationale(
                    "Smash-score names hide which resource is primary and make search/refactor harder. \
                    Split into composed facts (`RaceFacts`, `EntrantFacts`) instead of `RaceEntrantTransferClaim`."
                        .to_string(),
                )
                .with_locations(vec![
                    Location::new(path, symbol.start_line, symbol.end_line)
                        .with_snippet(symbol.source_text.clone())
                        .with_context(format!("{}: {}", symbol.kind.as_str(), symbol.name)),
                ])
                .with_suggested_fix(suggested_fix)
                .with_related_concepts(tokens)
                .with_prune_action(action)
                .with_action_summary(action_summary),
            );
        }
    }
    findings
}

fn pick_canonical_name(names: &[String]) -> String {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for name in names {
        let base = extract_base_name(name);
        *counts.entry(base).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, count)| *count)
        .map(|(name, _)| capitalize(&name))
        .unwrap_or_else(|| names[0].clone())
}

fn smash_score(name: &str) -> usize {
    let tokens = split_camel_tokens(name);
    let stripped = tokens
        .iter()
        .filter(|token| {
            !matches!(
                token.as_str(),
                "Props" | "Data" | "Info" | "Type" | "Args" | "Row" | "State" | "Wire" | "Facts"
                    | "Config" | "DTO" | "Model" | "Entity" | "Fields" | "Input" | "Output"
            )
        })
        .count();
    stripped
}

fn split_camel_tokens(name: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in name.chars() {
        if ch.is_uppercase() && !current.is_empty() {
            tokens.push(current);
            current = String::new();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn suggest_smashed_name(tokens: &[String]) -> String {
    if tokens.is_empty() {
        return "Facts".to_string();
    }
    if tokens.len() == 1 {
        return format!("{}Facts", tokens[0]);
    }
    format!("{}Facts", tokens[0])
}

fn field_names_preview(bindings: &[crate::patterns::interface_sprawl::type_shape::FieldBinding]) -> String {
    bindings
        .iter()
        .take(6)
        .map(|binding| binding.name.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => value.to_string(),
    }
}
