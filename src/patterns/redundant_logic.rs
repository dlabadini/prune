//! Redundant Logic detection - semantically similar code blocks

use super::{Finding, Location, PatternDetector, PatternType, Severity};
use crate::config::Config;
use crate::db::{Database, Symbol, SymbolKind};
use similar::TextDiff;

pub struct RedundantLogicDetector;

impl PatternDetector for RedundantLogicDetector {
    fn pattern_type(&self) -> PatternType {
        PatternType::RedundantLogic
    }

    fn detect(&self, db: &Database, config: &Config) -> Vec<Finding> {
        let mut findings = Vec::new();

        let similarity_threshold = config.effective_redundant_logic_similarity();

        // Get all function definitions
        let functions = match db.get_symbols_by_kind(SymbolKind::Function) {
            Ok(funcs) => funcs,
            Err(_) => return findings,
        };

        // Skip if too few functions
        if functions.len() < 2 {
            return findings;
        }

        // Find similar function pairs
        let similar_pairs = find_similar_pairs(&functions, similarity_threshold, db);

        for (a, b, similarity) in similar_pairs {
            let a_path = match db.get_file_path(a.file_id) {
                Ok(Some(path)) => path,
                _ => continue,
            };
            let b_path = match db.get_file_path(b.file_id) {
                Ok(Some(path)) => path,
                _ => continue,
            };

            // Skip if in same file and adjacent (might be intentional overloads)
            if a.file_id == b.file_id && (a.end_line + 5 >= b.start_line || b.end_line + 5 >= a.start_line) {
                continue;
            }

            let severity = calculate_severity(similarity, a.file_id != b.file_id);
            let confidence = similarity;

            let title = format!(
                "Similar functions: {} and {}",
                a.name, b.name
            );

            let description = format!(
                "`{}` and `{}` have {:.0}% structural similarity. \
                This may indicate duplicated logic that could be consolidated.",
                a.name, b.name, similarity * 100.0
            );

            let rationale = String::from(
                "Duplicated logic leads to:\n\
                - Bug fixes needing to be applied in multiple places\n\
                - Inconsistencies when one copy is updated but not the other\n\
                - Increased maintenance burden\n\
                - Confusion about which version to use or modify"
            );

            let locations = vec![
                Location::new(a_path.clone(), a.start_line, a.end_line)
                    .with_snippet(truncate_snippet(&a.source_text, 10))
                    .with_context(format!("function {}", a.name)),
                Location::new(b_path.clone(), b.start_line, b.end_line)
                    .with_snippet(truncate_snippet(&b.source_text, 10))
                    .with_context(format!("function {}", b.name)),
            ];

            let suggested_fix = generate_fix_suggestion(&a, &b, &a_path, &b_path, similarity);

            let finding = Finding::new(
                PatternType::RedundantLogic,
                severity,
                confidence,
                title,
                description,
            )
            .with_rationale(rationale)
            .with_locations(locations)
            .with_suggested_fix(suggested_fix)
            .with_related_concepts(vec![a.name.clone(), b.name.clone()]);

            findings.push(finding);
        }

        findings
    }
}

fn find_similar_pairs(
    functions: &[Symbol],
    threshold: f32,
    _db: &Database,
) -> Vec<(Symbol, Symbol, f32)> {
    let mut pairs = Vec::new();

    for (i, a) in functions.iter().enumerate() {
        // Skip very short functions (likely trivial)
        if a.source_text.lines().count() < 3 {
            continue;
        }

        for b in functions.iter().skip(i + 1) {
            // Skip very short functions
            if b.source_text.lines().count() < 3 {
                continue;
            }

            // Skip if line count differs by more than 2x (likely not similar)
            let a_lines = a.source_text.lines().count();
            let b_lines = b.source_text.lines().count();
            if a_lines > b_lines * 2 || b_lines > a_lines * 2 {
                continue;
            }

            let similarity = calculate_function_similarity(a, b);
            if similarity >= threshold {
                pairs.push((a.clone(), b.clone(), similarity));
            }
        }
    }

    // Sort by similarity (highest first)
    pairs.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    // Limit to avoid overwhelming output
    pairs.truncate(20);

    pairs
}

fn calculate_function_similarity(a: &Symbol, b: &Symbol) -> f32 {
    let a_normalized = normalize_function_body(&a.source_text);
    let b_normalized = normalize_function_body(&b.source_text);

    let diff = TextDiff::from_lines(&a_normalized, &b_normalized);
    diff.ratio()
}

fn normalize_function_body(source: &str) -> String {
    source
        .lines()
        .map(|line| {
            let trimmed = line.trim();
            // Remove comments
            if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with("*") {
                return String::new();
            }
            // Normalize whitespace
            trimmed.to_string()
        })
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        // Normalize common patterns
        .replace("const ", "let ")
        .replace("async ", "")
        .replace("await ", "")
        // Remove string literals (they differ but logic might be same)
        .chars()
        .filter(|c| *c != '"' && *c != '\'' && *c != '`')
        .collect()
}

fn calculate_severity(similarity: f32, different_files: bool) -> Severity {
    let base_severity = if similarity > 0.95 {
        Severity::High
    } else if similarity > 0.90 {
        Severity::Medium
    } else {
        Severity::Low
    };

    // Bump up severity if in different files (harder to notice)
    if different_files && base_severity == Severity::Medium {
        Severity::High
    } else {
        base_severity
    }
}

fn truncate_snippet(source: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = source.lines().collect();
    if lines.len() <= max_lines {
        source.to_string()
    } else {
        let truncated: Vec<&str> = lines.iter().take(max_lines).copied().collect();
        format!("{}\n// ... {} more lines", truncated.join("\n"), lines.len() - max_lines)
    }
}

fn generate_fix_suggestion(a: &Symbol, b: &Symbol, a_path: &str, b_path: &str, similarity: f32) -> String {
    let _same_file = a_path == b_path;

    if similarity > 0.95 {
        format!(
            r#"These functions are nearly identical ({:.0}% similar).

1. **Choose one to keep** - typically the one with:
   - Better name
   - More complete implementation
   - Better documentation

2. **Delete the duplicate** and update all call sites

3. **If both are needed for different modules**:
   - Move to a shared utilities location
   - Import from both places

Functions:
- `{}` in `{}`
- `{}` in `{}`"#,
            similarity * 100.0,
            a.name, extract_filename(a_path),
            b.name, extract_filename(b_path)
        )
    } else {
        format!(
            r#"These functions have significant overlap ({:.0}% similar).

1. **Identify the differences**:
   - Compare parameter types
   - Check for additional logic in either version
   - Look for different edge case handling

2. **Consolidation options**:
   a) **Extract common logic** into a helper function
   b) **Parameterize differences** with options/config
   c) **Create a base function** that both can call

3. **Example parameterization**:
   ```typescript
   function {}(input: T, options?: {{ mode?: 'a' | 'b' }}) {{
     // Common logic here
     if (options?.mode === 'b') {{
       // Variant B specific logic
     }}
   }}
   ```

Functions to review:
- `{}` ({} lines) in `{}`
- `{}` ({} lines) in `{}`"#,
            similarity * 100.0,
            infer_consolidated_name(&a.name, &b.name),
            a.name, a.source_text.lines().count(), extract_filename(a_path),
            b.name, b.source_text.lines().count(), extract_filename(b_path)
        )
    }
}

fn extract_filename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn infer_consolidated_name(a: &str, b: &str) -> String {
    // Find common prefix
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();

    let common_len = a_chars.iter()
        .zip(b_chars.iter())
        .take_while(|(a, b)| a == b)
        .count();

    if common_len >= 3 {
        a_chars[..common_len].iter().collect()
    } else {
        // Use the shorter name
        if a.len() < b.len() { a.to_string() } else { b.to_string() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_infer_consolidated_name() {
        assert_eq!(infer_consolidated_name("getUserById", "getUserByEmail"), "getUserBy");
        assert_eq!(infer_consolidated_name("fetch", "get"), "get");
    }

    #[test]
    fn test_normalize_function_body() {
        let source = r#"
        // Comment
        async function foo() {
            const x = "hello";
            await bar();
        }
        "#;
        let normalized = normalize_function_body(source);
        assert!(!normalized.contains("//"));
        assert!(!normalized.contains("async"));
    }
}
