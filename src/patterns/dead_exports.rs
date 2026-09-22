//! Dead Exports detection - exported symbols with no internal consumers

use super::{Finding, Location, PatternDetector, PatternType, Severity};
use crate::config::Config;
use crate::db::Database;

pub struct DeadExportsDetector;

impl PatternDetector for DeadExportsDetector {
    fn pattern_type(&self) -> PatternType {
        PatternType::DeadExports
    }

    fn detect(&self, db: &Database, _config: &Config) -> Vec<Finding> {
        let mut findings = Vec::new();

        // Get all exported symbols
        let exported_symbols = match db.get_exported_symbols() {
            Ok(symbols) => symbols,
            Err(_) => return findings,
        };

        let mut dead_exports: Vec<_> = Vec::new();

        for symbol in exported_symbols {
            // Check if this symbol has any references
            let has_refs = match db.has_references(symbol.id) {
                Ok(has) => has,
                Err(_) => continue,
            };

            if !has_refs {
                // Get file path for this symbol
                let file_path = match db.get_file_path(symbol.file_id) {
                    Ok(Some(path)) => path,
                    _ => continue,
                };

                dead_exports.push((symbol, file_path));
            }
        }

        // Group by file for better reporting
        let mut by_file: std::collections::HashMap<String, Vec<_>> =
            std::collections::HashMap::new();
        for (symbol, path) in dead_exports {
            by_file.entry(path).or_default().push(symbol);
        }

        for (file_path, symbols) in by_file {
            if symbols.is_empty() {
                continue;
            }

            // Need several dead exports before this is worth acting on.
            if symbols.len() < 5 {
                continue;
            }

            let severity = calculate_severity(symbols.len());
            let confidence = 0.9;

            let symbol_names: Vec<_> = symbols.iter().map(|s| s.name.clone()).collect();
            let title = format!(
                "{} unused exports in {}",
                symbols.len(),
                extract_filename(&file_path)
            );

            let description = format!(
                "These exported symbols have no references from other files: {}",
                symbol_names.join(", ")
            );

            let rationale = String::from(
                "Unused exports increase bundle size, clutter the API surface, and can confuse developers. \
                If these are intentionally part of a public library API, consider marking them explicitly. \
                Otherwise, remove the export keyword or delete the code entirely."
            );

            let mut locations = Vec::new();
            for symbol in &symbols {
                let location = Location::new(file_path.clone(), symbol.start_line, symbol.end_line)
                    .with_context(format!("{}: {}", symbol.kind.as_str(), symbol.name));
                locations.push(location);
            }

            let suggested_fix = generate_fix_suggestion(&file_path, &symbol_names);

            let finding = Finding::new(
                PatternType::DeadExports,
                severity,
                confidence,
                title,
                description,
            )
            .with_rationale(rationale)
            .with_locations(locations)
            .with_suggested_fix(suggested_fix)
            .with_related_concepts(symbol_names);

            findings.push(finding);
        }

        findings
    }
}

fn calculate_severity(dead_export_count: usize) -> Severity {
    if dead_export_count > 10 {
        Severity::High
    } else if dead_export_count > 5 {
        Severity::Medium
    } else {
        Severity::Low
    }
}

fn extract_filename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn generate_fix_suggestion(file_path: &str, symbol_names: &[String]) -> String {
    format!(
        r#"For each unused export in `{}`:

1. **Verify it's truly unused**:
   - Check if it's used in tests (Prune only analyzes source files)
   - Check if it's part of a public library API
   - Search for dynamic imports or string-based access

2. **If unused internally but part of public API**:
   - Add a comment: `// @public-api` to mark it as intentional
   - Consider documenting why it exists

3. **If truly unused**:
   - Remove the `export` keyword to make it private, or
   - Delete the code entirely if the functionality is no longer needed

Unused exports to review:
{}

Run `npx ts-prune` for additional verification of unused exports."#,
        file_path,
        symbol_names
            .iter()
            .map(|n| format!("- `{}`", n))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_severity_calculation() {
        assert_eq!(calculate_severity(15), Severity::High);
        assert_eq!(calculate_severity(7), Severity::Medium);
        assert_eq!(calculate_severity(3), Severity::Low);
    }
}
