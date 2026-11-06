//! Suggest `.prunerc` exclude globs for noisy paths (generated code, tests, patches).

use crate::config::Config;
use crate::patterns::Finding;
use ignore::WalkBuilder;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ExcludeSuggestion {
    pub glob: String,
    pub reason: String,
    pub hits: usize,
    pub sample: Vec<String>,
}

struct Rule {
    glob: &'static str,
    reason: &'static str,
    matches: fn(&str) -> bool,
}

const RULES: &[Rule] = &[
    Rule {
        glob: "patches/**",
        reason: "vendor patch directories are usually not app code",
        matches: |path| path.starts_with("patches/") || path.contains("/patches/"),
    },
    Rule {
        glob: "**/_generated/**",
        reason: "generated code (e.g. Convex, OpenAPI) should not be refactored by hand",
        matches: |path| path.contains("/_generated/") || path.starts_with("_generated/"),
    },
    Rule {
        glob: "**/fixtures/**",
        reason: "test fixtures are sample code, often broken on purpose",
        matches: |path| path.starts_with("fixtures/") || path.contains("/fixtures/"),
    },
    Rule {
        glob: "**/*.test.ts",
        reason: "test files are often allowed to be large or redundant",
        matches: |path| path.ends_with(".test.ts"),
    },
    Rule {
        glob: "**/*.test.tsx",
        reason: "test files are often allowed to be large or redundant",
        matches: |path| path.ends_with(".test.tsx"),
    },
    Rule {
        glob: "**/*.integration.test.ts",
        reason: "integration tests are often allowed to be large or redundant",
        matches: |path| path.ends_with(".integration.test.ts"),
    },
    Rule {
        glob: "**/*.spec.ts",
        reason: "spec files are often allowed to be large or redundant",
        matches: |path| path.ends_with(".spec.ts"),
    },
    Rule {
        glob: "**/*.spec.tsx",
        reason: "spec files are often allowed to be large or redundant",
        matches: |path| path.ends_with(".spec.tsx"),
    },
];

/// Paths from findings that match a noisy-path rule but are not already excluded.
pub fn from_findings(findings: &[Finding], config: &Config) -> Vec<ExcludeSuggestion> {
    let paths: Vec<String> = finding_paths(findings).into_iter().collect();
    from_paths(&paths, config)
}

/// Walk the repo for source files and suggest excludes for paths that look noisy.
pub fn from_filesystem(root: &Path, config: &Config) -> Vec<ExcludeSuggestion> {
    let mut paths = Vec::new();
    let walker = WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();
    for entry in walker.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !matches!(
            ext,
            "ts" | "tsx" | "js" | "jsx" | "mts" | "mjs" | "cts" | "cjs"
        ) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        if config.excludes_path(&rel) {
            continue;
        }
        paths.push(rel);
    }
    from_paths(&paths, config)
}

fn finding_paths(findings: &[Finding]) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    for finding in findings {
        for location in &finding.locations {
            paths.insert(location.file.clone());
        }
    }
    paths
}

fn from_paths(paths: &[String], config: &Config) -> Vec<ExcludeSuggestion> {
    let mut buckets: BTreeMap<&'static str, (usize, Vec<String>)> = BTreeMap::new();

    for path in paths {
        if config.excludes_path(path) {
            continue;
        }
        for rule in RULES {
            if !(rule.matches)(path) {
                continue;
            }
            if config.covers_glob(rule.glob) {
                continue;
            }
            let entry = buckets.entry(rule.glob).or_insert((0, Vec::new()));
            entry.0 += 1;
            if entry.1.len() < 3 {
                entry.1.push(path.clone());
            }
        }
    }

    RULES
        .iter()
        .filter_map(|rule| {
            buckets.get(rule.glob).map(|(hits, sample)| ExcludeSuggestion {
                glob: rule.glob.to_string(),
                reason: rule.reason.to_string(),
                hits: *hits,
                sample: sample.clone(),
            })
        })
        .filter(|suggestion| suggestion.hits > 0)
        .collect()
}

pub fn print_suggestions(suggestions: &[ExcludeSuggestion]) {
    if suggestions.is_empty() {
        return;
    }
    println!();
    println!("Suggested .prunerc excludes (add any that apply):");
    for suggestion in suggestions {
        println!(
            "  \"{}\"  # {} ({} path{})",
            suggestion.glob,
            suggestion.reason,
            suggestion.hits,
            if suggestion.hits == 1 { "" } else { "s" }
        );
        for sample in &suggestion.sample {
            println!("    e.g. {}", sample);
        }
    }
    println!();
    println!("Edit .prunerc, then run `prune scan` again.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patterns::{Finding, PatternType, Severity};

    #[test]
    fn suggests_generated_and_patches_not_already_excluded() {
        let config = Config {
            strictness: crate::config::Strictness::Default,
            thresholds: crate::config::PatternThresholds::default(),
            exclude: vec!["node_modules/**".to_string()],
            ..Config::default()
        };
        let findings = vec![
            Finding::new(
                PatternType::GodFile,
                Severity::High,
                0.9,
                "x".into(),
                "y".into(),
            )
            .with_locations(vec![crate::patterns::Location::new(
                "convex/_generated/api.d.ts".to_string(),
                1,
                1,
            )]),
            Finding::new(
                PatternType::GodFile,
                Severity::High,
                0.9,
                "x".into(),
                "y".into(),
            )
            .with_locations(vec![crate::patterns::Location::new(
                "patches/foo/bar.js".to_string(),
                1,
                1,
            )]),
        ];
        let suggestions = from_findings(&findings, &config);
        let globs: Vec<_> = suggestions.iter().map(|s| s.glob.as_str()).collect();
        assert!(globs.contains(&"**/_generated/**"));
        assert!(globs.contains(&"patches/**"));
    }

    #[test]
    fn skips_rules_already_in_config() {
        let mut config = Config::default();
        config.exclude.push("patches/**".to_string());
        let paths = vec!["patches/embla/foo.js".to_string()];
        assert!(from_paths(&paths, &config).is_empty());
    }
}
