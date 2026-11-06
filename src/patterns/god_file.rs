//! God File detection - files with too many exports or excessive complexity

use super::{Finding, Location, PatternDetector, PatternType, Severity};
use crate::config::Config;
use crate::db::{Database, Symbol, SymbolKind};
use std::collections::{HashMap, HashSet};

pub struct GodFileDetector;

impl PatternDetector for GodFileDetector {
    fn pattern_type(&self) -> PatternType {
        PatternType::GodFile
    }

    fn detect(&self, db: &Database, config: &Config) -> Vec<Finding> {
        let mut findings = Vec::new();

        let max_exports = config.effective_god_file_max_exports();
        let max_lines = config.effective_god_file_max_lines();

        // Get export counts per file
        let file_stats = match db.count_exports_per_file() {
            Ok(stats) => stats,
            Err(_) => return findings,
        };
        let mut exported_by_file: HashMap<i64, Vec<Symbol>> = HashMap::new();
        if let Ok(symbols) = db.get_exported_symbols() {
            for symbol in symbols {
                exported_by_file.entry(symbol.file_id).or_default().push(symbol);
            }
        }

        for (file_id, path, line_count, export_count) in file_stats {
            let surface = export_surface(exported_by_file.get(&file_id).map(Vec::as_slice).unwrap_or(&[]));
            // Skip files that are obviously utility/helper files (they get a higher threshold)
            let is_utility_file = path.contains("utils")
                || path.contains("helpers")
                || path.contains("constants")
                || path.contains("types");

            let effective_max_exports = if is_utility_file {
                max_exports * 2
            } else {
                max_exports
            };

            let effective_max_lines = if is_utility_file {
                max_lines * 2
            } else {
                max_lines
            };

            // Behavior (functions, classes, stateful values) over the limit is a god file on its
            // own. Declarations (types, validators, `as const` data, a validator and its inferred
            // type counted once) only count when the file is also long.
            let exceeds_exports = surface.behavior > effective_max_exports
                || (surface.concepts > effective_max_exports && line_count > effective_max_lines / 2);
            // Line count alone is not a god file — large pages and schemas are common.
            let exceeds_lines = line_count > effective_max_lines
                && surface.concepts >= effective_max_exports.max(8);

            if exceeds_exports || exceeds_lines {
                let severity = calculate_severity(
                    export_count,
                    effective_max_exports,
                    line_count,
                    effective_max_lines,
                );

                let confidence = calculate_confidence(
                    export_count,
                    effective_max_exports,
                    line_count,
                    effective_max_lines,
                );

                let mut reasons = Vec::new();
                if exceeds_exports {
                    reasons.push(format!(
                        "{} exports, {} distinct concepts, {} with behavior (threshold: {})",
                        export_count, surface.concepts, surface.behavior, effective_max_exports
                    ));
                }
                if exceeds_lines {
                    reasons.push(format!(
                        "{} lines (threshold: {})",
                        line_count, effective_max_lines
                    ));
                }

                let title = format!("God file: {}", extract_filename(&path));
                let description = format!(
                    "This file has grown beyond maintainable size: {}",
                    reasons.join(", ")
                );

                let rationale = "Large files with many exports become difficult to maintain, test, and understand. \
                    Changes to one part of the file can unexpectedly affect other parts. \
                    Consider splitting into focused modules by domain or feature."
                    .to_string();

                let suggested_fix = generate_fix_suggestion(&path, export_count, line_count);

                let location = Location::new(path.clone(), 1, line_count);

                let finding = Finding::new(
                    PatternType::GodFile,
                    severity,
                    confidence,
                    title,
                    description,
                )
                .with_rationale(rationale)
                .with_locations(vec![location])
                .with_suggested_fix(suggested_fix);

                findings.push(finding);
            }
        }

        findings
    }
}

/// What a file's exports amount to once declarations are weighed fairly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExportSurface {
    /// Exports after pairing each type with the value it is inferred from.
    concepts: usize,
    /// Functions, classes, and values that are not declarative data.
    behavior: usize,
}

fn export_surface(symbols: &[Symbol]) -> ExportSurface {
    let values: HashSet<&str> = symbols
        .iter()
        .filter(|symbol| !is_type(symbol))
        .map(|symbol| symbol.name.as_str())
        .collect();
    let mut concepts = 0;
    let mut behavior = 0;
    for symbol in symbols {
        if is_type(symbol) && pairs_with_value(symbol, &values) {
            continue;
        }
        concepts += 1;
        if !is_type(symbol) && !is_declarative_value(symbol) {
            behavior += 1;
        }
    }
    ExportSurface { concepts, behavior }
}

fn is_type(symbol: &Symbol) -> bool {
    matches!(symbol.kind, SymbolKind::Interface | SymbolKind::TypeAlias)
}

/// `type Pool = Infer<typeof vPool>`, `type X = z.infer<typeof XSchema>`, or `const X` + `type X`.
fn pairs_with_value(symbol: &Symbol, values: &HashSet<&str>) -> bool {
    let name = symbol.name.as_str();
    if values.contains(name) {
        return true;
    }
    let candidates = [
        format!("v{name}"),
        format!("{name}Schema"),
        format!("{name}Validator"),
        format!("{}Schema", lower_first(name)),
    ];
    if candidates.iter().any(|candidate| values.contains(candidate.as_str())) {
        return true;
    }
    let text = symbol.source_text.as_str();
    text.match_indices("typeof ").any(|(index, _)| {
        let rest = &text[index + "typeof ".len()..];
        let ident: String = rest
            .chars()
            .take_while(|ch| ch.is_alphanumeric() || *ch == '_' || *ch == '$')
            .collect();
        values.contains(ident.as_str())
    })
}

fn lower_first(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Validators, schemas, literal tables, and `as const` tuples: declarations, not behavior.
fn is_declarative_value(symbol: &Symbol) -> bool {
    if symbol.kind != SymbolKind::Variable && symbol.kind != SymbolKind::Enum {
        return false;
    }
    if symbol.kind == SymbolKind::Enum || symbol.source_text.contains("as const") {
        return true;
    }
    match symbol.callee.as_deref() {
        // No call: a literal (string, number, array, object).
        None => true,
        Some(callee) => {
            let root = callee.split('.').next().unwrap_or(callee);
            matches!(root, "v" | "z" | "t" | "s" | "yup" | "Type" | "Schema" | "Object")
                || matches!(
                    callee.rsplit('.').next().unwrap_or(callee),
                    "literals" | "literal" | "union" | "object" | "defineTable" | "freeze" | "keys" | "fromEntries"
                )
        }
    }
}

fn calculate_severity(
    export_count: usize,
    max_exports: usize,
    line_count: usize,
    max_lines: usize,
) -> Severity {
    let export_ratio = export_count as f32 / max_exports as f32;
    let line_ratio = line_count as f32 / max_lines as f32;
    let max_ratio = export_ratio.max(line_ratio);

    if max_ratio > 3.0 {
        Severity::Critical
    } else if max_ratio > 2.0 {
        Severity::High
    } else if max_ratio > 1.5 {
        Severity::Medium
    } else {
        Severity::Low
    }
}

fn calculate_confidence(
    export_count: usize,
    max_exports: usize,
    line_count: usize,
    max_lines: usize,
) -> f32 {
    let export_ratio = export_count as f32 / max_exports as f32;
    let line_ratio = line_count as f32 / max_lines as f32;

    // Higher ratio = higher confidence this is truly a god file
    let ratio = export_ratio.max(line_ratio);

    if ratio > 2.0 {
        0.95
    } else if ratio > 1.5 {
        0.85
    } else {
        0.75
    }
}

fn extract_filename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn generate_fix_suggestion(path: &str, export_count: usize, _line_count: usize) -> String {
    let filename = extract_filename(path);
    let dir = path.rsplit_once('/').map(|(d, _)| d).unwrap_or(".");

    format!(
        r#"1. Analyze the {} exports in `{}` to identify logical groupings
2. Create new focused modules in `{}/` for each grouping
3. Move related functions, types, and constants to their new homes
4. Update imports in consuming files
5. Consider creating an index file that re-exports public API if needed

Example structure:
- `{}/core.ts` - Core logic
- `{}/types.ts` - Type definitions
- `{}/utils.ts` - Helper functions
- `{}/index.ts` - Public API re-exports"#,
        export_count, filename, dir, dir, dir, dir, dir
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_severity_calculation() {
        assert_eq!(calculate_severity(100, 20, 500, 500), Severity::Critical);
        assert_eq!(calculate_severity(50, 20, 500, 500), Severity::High);
        assert_eq!(calculate_severity(35, 20, 500, 500), Severity::Medium);
        assert_eq!(calculate_severity(25, 20, 500, 500), Severity::Low);
    }

    fn symbol(name: &str, kind: SymbolKind, source: &str, callee: Option<&str>) -> Symbol {
        Symbol {
            id: 0,
            file_id: 1,
            name: name.into(),
            kind,
            start_line: 1,
            end_line: 1,
            is_exported: true,
            is_default: false,
            source_text: source.into(),
            callee: callee.map(String::from),
        }
    }

    #[test]
    fn validator_and_inferred_type_count_once_and_are_declarative() {
        let mut symbols = Vec::new();
        for index in 0..20 {
            symbols.push(symbol(&format!("vPool{index}"), SymbolKind::Variable, "literals(\"a\", \"b\")", Some("literals")));
            symbols.push(symbol(
                &format!("Pool{index}"),
                SymbolKind::TypeAlias,
                &format!("type Pool{index} = Infer<typeof vPool{index}>"),
                None,
            ));
        }
        symbols.push(symbol("POOLS", SymbolKind::Variable, "[\"a\", \"b\"] as const", None));
        let surface = export_surface(&symbols);
        assert_eq!(surface, ExportSurface { concepts: 21, behavior: 0 });
    }

    #[test]
    fn functions_are_behavior() {
        let symbols: Vec<Symbol> = (0..25)
            .map(|index| symbol(&format!("f{index}"), SymbolKind::Function, "function f() {}", None))
            .collect();
        assert_eq!(export_surface(&symbols).behavior, 25);
    }

    #[test]
    fn test_extract_filename() {
        assert_eq!(extract_filename("src/utils/helpers.ts"), "helpers.ts");
        assert_eq!(extract_filename("helpers.ts"), "helpers.ts");
    }
}
