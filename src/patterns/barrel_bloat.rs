//! Barrel Bloat detection - index files re-exporting too many modules

use super::{Finding, Location, PatternDetector, PatternType, Severity};
use crate::config::Config;
use crate::db::Database;

pub struct BarrelBloatDetector;

impl PatternDetector for BarrelBloatDetector {
    fn pattern_type(&self) -> PatternType {
        PatternType::BarrelBloat
    }

    fn detect(&self, db: &Database, config: &Config) -> Vec<Finding> {
        let mut findings = Vec::new();

        let threshold = config.effective_barrel_bloat_threshold();

        // Get reexport counts for index files
        let reexport_counts = match db.get_reexport_counts() {
            Ok(counts) => counts,
            Err(_) => return findings,
        };

        for (_file_id, path, import_count) in reexport_counts {
            if import_count > threshold {
                let severity = calculate_severity(import_count, threshold);
                let confidence = calculate_confidence(import_count, threshold);

                let title = format!("Barrel bloat: {}", extract_filename(&path));
                let description = format!(
                    "This index file re-exports {} modules (threshold: {}). \
                    Large barrel files can cause bundle size issues and slow TypeScript compilation.",
                    import_count, threshold
                );

                let rationale = "Barrel files (index.ts) that re-export many modules can:\n\
                    - Cause unnecessary code to be bundled when any export is imported\n\
                    - Slow down TypeScript compilation and IDE responsiveness\n\
                    - Make it harder to tree-shake unused code\n\
                    - Hide the true dependency graph of your application"
                    .to_string();

                let suggested_fix = generate_fix_suggestion(&path, import_count);

                let location = Location::new(path.clone(), 1, 1)
                    .with_context(format!("Re-exports {} modules", import_count));

                let finding = Finding::new(
                    PatternType::BarrelBloat,
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

fn calculate_severity(import_count: usize, threshold: usize) -> Severity {
    let ratio = import_count as f32 / threshold as f32;

    if ratio > 3.0 {
        Severity::High
    } else if ratio > 2.0 {
        Severity::Medium
    } else {
        Severity::Low
    }
}

fn calculate_confidence(import_count: usize, threshold: usize) -> f32 {
    let ratio = import_count as f32 / threshold as f32;

    if ratio > 2.0 {
        0.90
    } else if ratio > 1.5 {
        0.80
    } else {
        0.70
    }
}

fn extract_filename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn generate_fix_suggestion(path: &str, import_count: usize) -> String {
    let dir = path.rsplit_once('/').map(|(d, _)| d).unwrap_or(".");

    format!(
        r#"Options to reduce barrel bloat:

1. **Direct imports**: Update consumers to import directly from source files instead of the barrel:
   ```typescript
   // Before: import {{ Component }} from '{dir}';
   // After:  import {{ Component }} from '{dir}/Component';
   ```

2. **Split by domain**: Create sub-barrels for logical groupings:
   ```
   {dir}/
   ├── components/index.ts
   ├── hooks/index.ts
   ├── utils/index.ts
   └── index.ts (re-exports only truly public API)
   ```

3. **Evaluate necessity**: Does everything need to be exported? Remove internal-only exports.

4. **Use path aliases**: Configure path aliases to make direct imports ergonomic:
   ```json
   {{ "paths": {{ "@{module}/*": ["{dir}/*"] }} }}
   ```

Currently re-exporting {} modules - consider reducing to <{} for better performance."#,
        import_count,
        import_count / 2,
        dir = dir,
        module = dir.rsplit('/').next().unwrap_or("module")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_severity_calculation() {
        // ratio > 3.0 = High, ratio > 2.0 = Medium, else Low
        assert_eq!(calculate_severity(80, 20), Severity::High); // 4.0 > 3.0
        assert_eq!(calculate_severity(60, 20), Severity::Medium); // 3.0 not > 3.0
        assert_eq!(calculate_severity(45, 20), Severity::Medium); // 2.25 > 2.0
        assert_eq!(calculate_severity(25, 20), Severity::Low); // 1.25 not > 2.0
    }
}
