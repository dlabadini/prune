//! Lexicon cleanliness — naming drift on shared shapes and resource smash in identifiers.

use super::{Finding, Location, PatternDetector, PatternType, Severity};
use crate::compose::{compose_from_lexicon, format_action_markdown, format_action_summary};
use crate::config::Config;
use crate::db::{Database, Symbol, SymbolKind};
use crate::patterns::interface_sprawl::{
    extract_base_name, index_types, label_boundary, shape_symbols, type_shape::bundle_signature,
    IndexedType,
};
use std::collections::{HashMap, HashSet};

pub struct LexiconDetector;

impl PatternDetector for LexiconDetector {
    fn pattern_type(&self) -> PatternType {
        PatternType::Lexicon
    }

    fn detect(&self, db: &Database, config: &Config) -> Vec<Finding> {
        let symbols = shape_symbols(db);
        if symbols.is_empty() {
            return Vec::new();
        }

        let indexed = index_types(&symbols);
        let min_fields = config.effective_lexicon_min_fields();
        let max_smash = config.effective_lexicon_max_smash_score();
        let max_findings = config.effective_lexicon_max_findings();

        let mut findings = Vec::new();
        findings.extend(detect_homonym_clusters(db, config, &indexed, min_fields));
        findings.extend(detect_glossary_conflicts(db, config, &indexed));
        if config.lexicon.smash_names {
            findings.extend(detect_smash_names(db, &symbols, max_smash));
        }

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
    config: &Config,
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

    // Split each group at fenced boundaries.
    let mut groups: Vec<(String, Vec<usize>, Option<String>)> = Vec::new();
    for (signature, indexes) in shape_groups {
        let paths: Vec<String> = indexes
            .iter()
            .map(|index| {
                db.get_file_path(types[*index].symbol.file_id)
                    .ok()
                    .flatten()
                    .unwrap_or_default()
            })
            .collect();
        let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
        for (label, members) in config.partition_by_boundary(&refs) {
            groups.push((
                signature.clone(),
                members.iter().map(|member| indexes[*member]).collect(),
                label,
            ));
        }
    }

    let mut findings = Vec::new();
    for (signature, indexes, boundary) in groups {
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
        if let Some(finding) = findings.pop() {
            findings.push(label_boundary(finding, boundary));
        }
    }
    findings
}

/// A glossary term that names unrelated shapes: one word, two concepts.
fn detect_glossary_conflicts(db: &Database, config: &Config, types: &[IndexedType]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (term, meaning) in &config.lexicon.glossary {
        let wanted = term.to_ascii_lowercase();
        let users: Vec<&IndexedType> = types
            .iter()
            .filter(|ty| {
                split_identifier_tokens(&ty.symbol.name)
                    .iter()
                    .any(|token| token.to_ascii_lowercase() == wanted)
            })
            .collect();
        if users.len() < 2 {
            continue;
        }
        // Group by field overlap: shapes sharing at least a third of their fields are one concept.
        let field_sets: Vec<HashSet<&str>> = users
            .iter()
            .map(|ty| ty.bindings.iter().map(|binding| binding.name.as_str()).collect())
            .collect();
        let mut group_of: Vec<usize> = (0..users.len()).collect();
        for left in 0..users.len() {
            for right in left + 1..users.len() {
                let shared = field_sets[left].intersection(&field_sets[right]).count();
                let union = field_sets[left].union(&field_sets[right]).count().max(1);
                if shared * 3 >= union {
                    let (from, to) = (group_of[right], group_of[left]);
                    for group in group_of.iter_mut() {
                        if *group == from {
                            *group = to;
                        }
                    }
                }
            }
        }
        let mut concepts: Vec<Vec<usize>> = Vec::new();
        let mut seen: HashMap<usize, usize> = HashMap::new();
        for (index, group) in group_of.iter().enumerate() {
            let slot = *seen.entry(*group).or_insert_with(|| {
                concepts.push(Vec::new());
                concepts.len() - 1
            });
            concepts[slot].push(index);
        }
        if concepts.len() < 2 {
            continue;
        }
        let mut locations = Vec::new();
        let mut lines = Vec::new();
        let mut names = Vec::new();
        for (number, members) in concepts.iter().enumerate() {
            let member_names: Vec<String> = members
                .iter()
                .map(|index| users[*index].symbol.name.clone())
                .collect();
            let fields = field_names_preview(&users[members[0]].bindings);
            lines.push(format!("{}. {} — fields: {}", number + 1, member_names.join(", "), fields));
            names.extend(member_names);
            for index in members {
                let ty = users[*index];
                if let Ok(Some(path)) = db.get_file_path(ty.symbol.file_id) {
                    locations.push(
                        Location::new(path, ty.symbol.start_line, ty.symbol.end_line)
                            .with_context(format!("{}: {}", ty.symbol.kind.as_str(), ty.symbol.name)),
                    );
                }
            }
        }
        findings.push(
            Finding::new(
                PatternType::Lexicon,
                Severity::Medium,
                0.7,
                format!("`{term}` names {} different concepts", concepts.len()),
                format!(
                    "Glossary: {term} = {meaning}.\nThese shapes share the term but not their fields:\n{}",
                    lines.join("\n")
                ),
            )
            .with_rationale(
                "One term for two concepts makes every reader stop and check which one is meant. \
                 Keep the glossary meaning for the shapes that match it and rename the others."
                    .to_string(),
            )
            .with_locations(locations)
            .with_suggested_fix(format!(
                "Keep `{term}` for the concept that means \"{meaning}\". Rename the other group(s) to a term \
                 that says what they are, and add that term to the glossary."
            ))
            .with_related_concepts({
                let mut concepts = vec![format!("glossary:{term}")];
                concepts.extend(names);
                concepts
            }),
        );
    }
    findings
}

fn detect_smash_names(db: &Database, symbols: &[Symbol], max_smash: usize) -> Vec<Finding> {
    let mut findings = Vec::new();
    for symbol in symbols {
        // `_PortConformsToSdk`: compile-time assertions, not domain names.
        if symbol.name.starts_with('_') || symbol.kind == SymbolKind::Variable {
            continue;
        }
        let score = smash_score(&symbol.name);
        if score <= max_smash {
            continue;
        }
        if symbol.name.ends_with("Ctx") || symbol.name.ends_with("Context") {
            continue;
        }
        let tokens = split_identifier_tokens(&symbol.name);
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

/// The most common base among `names`, spelled as the code spells it (`StandingFact`, never
/// `Standingfact`). Ties go to the shortest, then alphabetical, so the pick is stable.
fn pick_canonical_name(names: &[String]) -> String {
    let mut groups: HashMap<String, Vec<String>> = HashMap::new();
    for name in names {
        groups
            .entry(extract_base_name(name))
            .or_default()
            .push(cased_base_name(name));
    }
    groups
        .into_values()
        .max_by(|left, right| {
            left.len().cmp(&right.len()).then_with(|| {
                let l = left.iter().min_by_key(|n| (n.len(), (*n).clone()));
                let r = right.iter().min_by_key(|n| (n.len(), (*n).clone()));
                r.cmp(&l)
            })
        })
        .and_then(|spellings| spellings.into_iter().min_by_key(|n| (n.len(), n.clone())))
        .map(|name| capitalize(&name))
        .unwrap_or_else(|| names[0].clone())
}

/// `extract_base_name` without lowercasing: strip one conventional suffix.
fn cased_base_name(name: &str) -> String {
    let lowered = extract_base_name(name);
    name.get(..lowered.len()).unwrap_or(name).to_string()
}

fn smash_score(name: &str) -> usize {
    let tokens = split_identifier_tokens(name);
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

/// Split an identifier into words, keeping acronyms whole:
/// `ThreadUIAction` → Thread, UI, Action; `HTTPServer2Error` → HTTP, Server2, Error;
/// `snake_case_name` → snake, case, name.
pub fn split_identifier_tokens(name: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for part in name.split(['_', '-', '$']).filter(|part| !part.is_empty()) {
        let chars: Vec<char> = part.chars().collect();
        let mut current = String::new();
        for (index, ch) in chars.iter().enumerate() {
            let prev = index.checked_sub(1).map(|i| chars[i]);
            let next = chars.get(index + 1);
            let boundary = ch.is_uppercase()
                && !current.is_empty()
                && (prev.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit())
                    || (prev.is_some_and(|p| p.is_uppercase()) && next.is_some_and(|n| n.is_lowercase())));
            if boundary {
                tokens.push(std::mem::take(&mut current));
            }
            current.push(*ch);
        }
        if !current.is_empty() {
            tokens.push(current);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_keep_acronyms() {
        assert_eq!(split_identifier_tokens("ThreadUIAction"), vec!["Thread", "UI", "Action"]);
        assert_eq!(split_identifier_tokens("HTTPServerError"), vec!["HTTP", "Server", "Error"]);
        assert_eq!(split_identifier_tokens("_PortConformsToSdk"), vec!["Port", "Conforms", "To", "Sdk"]);
        assert_eq!(smash_score("ThreadUIAction"), 3);
    }

    #[test]
    fn canonical_name_keeps_the_code_spelling() {
        let names = vec![
            "StandingFact".to_string(),
            "StandingFactRow".to_string(),
            "ParticipantStanding".to_string(),
        ];
        assert_eq!(pick_canonical_name(&names), "StandingFact");
    }
}
