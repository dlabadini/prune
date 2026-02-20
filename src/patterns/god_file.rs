//! God File detection - files with too many exports or excessive complexity

use super::{Finding, Location, PatternDetector, PatternType, Severity};
use crate::config::Config;
use crate::db::Database;

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

        for (_file_id, path, line_count, export_count) in file_stats {
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

            let exceeds_exports = export_count > effective_max_exports;
            let exceeds_lines = line_count > effective_max_lines;

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
                        "{} exports (threshold: {})",
                        export_count, effective_max_exports
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

    #[test]
    fn test_extract_filename() {
        assert_eq!(extract_filename("src/utils/helpers.ts"), "helpers.ts");
        assert_eq!(extract_filename("helpers.ts"), "helpers.ts");
    }
}
