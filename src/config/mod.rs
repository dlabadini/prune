//! Configuration management for Prune

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    ReadError(#[from] std::io::Error),
    #[error("Failed to parse config: {0}")]
    ParseError(#[from] toml::de::Error),
    #[error("Failed to serialize config: {0}")]
    SerializeError(#[from] toml::ser::Error),
}

/// Strictness level for pattern detection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Strictness {
    /// Fewer findings, higher thresholds
    Relaxed,
    /// Balanced defaults for small-to-mid teams
    #[default]
    Default,
    /// More findings, lower thresholds
    Strict,
}

impl Strictness {
    pub fn cycle(self) -> Self {
        match self {
            Strictness::Relaxed => Strictness::Default,
            Strictness::Default => Strictness::Strict,
            Strictness::Strict => Strictness::Relaxed,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Strictness::Relaxed => "relaxed",
            Strictness::Default => "default",
            Strictness::Strict => "strict",
        }
    }
}

/// Pattern-specific threshold overrides
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PatternThresholds {
    /// Similarity threshold for interface sprawl detection (0.0-1.0)
    #[serde(default)]
    pub interface_sprawl_similarity: Option<f32>,
    /// Minimum cluster size for interface sprawl
    #[serde(default)]
    pub interface_sprawl_min_cluster: Option<usize>,
    /// Minimum shared fields in a composable bundle
    #[serde(default)]
    pub interface_sprawl_min_bundle_fields: Option<usize>,
    /// Minimum overlap ratio for partial-overlap clusters (0.0-1.0)
    #[serde(default)]
    pub interface_sprawl_min_overlap_ratio: Option<f32>,
    /// Maximum interface sprawl findings per scan
    #[serde(default)]
    pub interface_sprawl_max_findings: Option<usize>,
    /// Minimum shared fields for lexicon homonym detection
    #[serde(default)]
    pub lexicon_min_fields: Option<usize>,
    /// Smash-score above this flags a compound identifier (token count)
    #[serde(default)]
    pub lexicon_max_smash_score: Option<usize>,
    /// Maximum lexicon findings per scan
    #[serde(default)]
    pub lexicon_max_findings: Option<usize>,
    /// Enable semantic embeddings (auto when model is cached unless set explicitly)
    #[serde(default)]
    pub embeddings_enabled: Option<bool>,
    /// Similarity threshold for redundant logic detection
    #[serde(default)]
    pub redundant_logic_similarity: Option<f32>,
    /// Maximum exports before flagging as god file
    #[serde(default)]
    pub god_file_max_exports: Option<usize>,
    /// Maximum lines before flagging as god file
    #[serde(default)]
    pub god_file_max_lines: Option<usize>,
    /// Threshold for barrel bloat detection
    #[serde(default)]
    pub barrel_bloat_threshold: Option<usize>,
    /// Threshold for dead export detection grace period (days)
    #[serde(default)]
    pub dead_export_grace_days: Option<u32>,
    /// Report a file once it has at least this many dead exports (default 1)
    #[serde(default)]
    pub dead_exports_min_per_file: Option<usize>,
}

/// Where the program starts. Anything not reachable from an entry is dead.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntriesConfig {
    /// Extra entry globs (repo-relative). Every export of an entry file counts as used.
    #[serde(default = "default_entry_files")]
    pub files: Vec<String>,
    /// Globs for test files. They are indexed only as reference sources, even when `exclude`
    /// matches them, so an export used only by tests is reported as test-only, not unused.
    #[serde(default = "default_test_files")]
    pub test_files: Vec<String>,
    /// Read test files for references (set false to ignore tests entirely).
    #[serde(default = "default_true")]
    pub index_tests: bool,
    /// Treat every export of a `"private": true` package entry as public API.
    #[serde(default)]
    pub private_package_exports_are_api: bool,
}

impl Default for EntriesConfig {
    fn default() -> Self {
        Self {
            files: default_entry_files(),
            test_files: default_test_files(),
            index_tests: true,
            private_package_exports_are_api: false,
        }
    }
}

/// Framework adapters: entry conventions expressed as data.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FrameworksConfig {
    #[serde(default)]
    pub next: NextConfig,
    #[serde(default)]
    pub convex: ConvexConfig,
}

/// Next.js App Router conventions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NextConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// App roots (directories holding `next.config.*`). Empty = every `next.config.*` found.
    #[serde(default)]
    pub roots: Vec<String>,
    /// File stems under `app/` that Next loads (`page.tsx`, `route.ts`, ...).
    #[serde(default = "default_next_app_entries")]
    pub app_entries: Vec<String>,
    /// File stems at the app root (or `src/`) that Next loads.
    #[serde(default = "default_next_root_entries")]
    pub root_entries: Vec<String>,
}

impl Default for NextConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            roots: Vec::new(),
            app_entries: default_next_app_entries(),
            root_entries: default_next_root_entries(),
        }
    }
}

/// Convex conventions: function modules, generated `api`/`internal` references.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvexConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Convex function directories. Empty = `convex.json` `functions`, else `convex/`.
    #[serde(default)]
    pub dirs: Vec<String>,
    /// Exact builder names that register a Convex function.
    #[serde(default = "default_convex_builders")]
    pub function_builders: Vec<String>,
    /// Builder name suffixes that register a Convex function (`adminQuery`, `manageMutation`).
    #[serde(default = "default_convex_builder_suffixes")]
    pub builder_suffixes: Vec<String>,
    /// Module names (relative to a Convex dir, no extension) Convex itself loads.
    #[serde(default = "default_convex_entry_modules")]
    pub entry_modules: Vec<String>,
}

impl Default for ConvexConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            dirs: Vec::new(),
            function_builders: default_convex_builders(),
            builder_suffixes: default_convex_builder_suffixes(),
            entry_modules: default_convex_entry_modules(),
        }
    }
}

impl ConvexConfig {
    /// Whether `callee` (as written: `query`, `zCustomQuery`, `ctx.adminQuery`) registers a function.
    pub fn is_function_builder(&self, callee: &str) -> bool {
        let last = callee.rsplit('.').next().unwrap_or(callee);
        if self.function_builders.iter().any(|name| name == last) {
            return true;
        }
        let lower = last.to_ascii_lowercase();
        self.builder_suffixes
            .iter()
            .any(|suffix| lower.ends_with(&suffix.to_ascii_lowercase()) && lower.len() > suffix.len())
    }
}

/// Which dead-code classes become findings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeadCodeConfig {
    /// Report files no entry reaches.
    #[serde(default = "default_true")]
    pub report_unreachable_files: bool,
    /// Report exports only tests use.
    #[serde(default = "default_true")]
    pub report_test_only: bool,
    /// Report exports only used inside their own file (drop the `export`).
    #[serde(default = "default_true")]
    pub report_same_file: bool,
}

impl Default for DeadCodeConfig {
    fn default() -> Self {
        Self {
            report_unreachable_files: true,
            report_test_only: true,
            report_same_file: true,
        }
    }
}

/// Main configuration struct
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Overall strictness level
    #[serde(default)]
    pub strictness: Strictness,
    /// Pattern-specific threshold overrides
    #[serde(default)]
    pub thresholds: PatternThresholds,
    /// File patterns to exclude (in addition to .gitignore)
    #[serde(default = "default_exclude")]
    pub exclude: Vec<String>,
    /// File patterns to include (overrides exclude)
    #[serde(default)]
    pub include: Vec<String>,
    /// Patterns to disable entirely
    #[serde(default)]
    pub disabled_patterns: Vec<String>,
    /// Entry points for reachability
    #[serde(default)]
    pub entries: EntriesConfig,
    /// Framework adapters (Next.js, Convex)
    #[serde(default)]
    pub frameworks: FrameworksConfig,
    /// Dead code reporting
    #[serde(default)]
    pub dead_exports: DeadCodeConfig,
    /// Lexicon: glossary and naming heuristics
    #[serde(default)]
    pub lexicon: LexiconConfig,
    /// Named path groups. A fenced boundary owns its types: duplicate shapes across its fence
    /// are intentional and are not reported; duplicates inside it are.
    #[serde(default)]
    pub boundaries: Vec<Boundary>,
}

/// A named group of paths (a package, a layer) with its own rules.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Boundary {
    pub name: String,
    pub paths: Vec<String>,
    /// Owns its types; cross-boundary duplicate shapes are intentional.
    #[serde(default)]
    pub fenced: bool,
}

/// Lexicon settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LexiconConfig {
    /// Term → meaning. A term used in names of two unrelated shapes is flagged.
    #[serde(default)]
    pub glossary: std::collections::BTreeMap<String, String>,
    /// Flag identifiers that pack many resource tokens (`RaceEntrantTransferClaim`). Off by
    /// default: names like `ThreadUIAction` are fine.
    #[serde(default)]
    pub smash_names: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            strictness: Strictness::Default,
            thresholds: PatternThresholds::default(),
            exclude: default_exclude(),
            include: vec![],
            disabled_patterns: vec![],
            entries: EntriesConfig::default(),
            frameworks: FrameworksConfig::default(),
            dead_exports: DeadCodeConfig::default(),
            lexicon: LexiconConfig::default(),
            boundaries: Vec::new(),
        }
    }
}

impl Config {
    /// Load config from file, or return defaults if not found
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            Ok(toml::from_str(&content)?)
        } else {
            Ok(Self::default())
        }
    }

    /// Find and load config from standard locations
    pub fn find_and_load(project_root: &Path) -> Result<Self, ConfigError> {
        // Check for .prunerc first
        let prunerc = project_root.join(".prunerc");
        if prunerc.exists() {
            return Self::load(&prunerc);
        }

        // Check for prune.config.toml
        let config_toml = project_root.join("prune.config.toml");
        if config_toml.exists() {
            return Self::load(&config_toml);
        }

        // Return defaults
        Ok(Self::default())
    }

    /// Save config to file
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Get effective threshold values based on strictness
    pub fn effective_interface_sprawl_similarity(&self) -> f32 {
        self.thresholds
            .interface_sprawl_similarity
            .unwrap_or(match self.strictness {
                Strictness::Relaxed => 0.90,
                Strictness::Default => 0.85,
                Strictness::Strict => 0.80,
            })
    }

    pub fn effective_interface_sprawl_min_cluster(&self) -> usize {
        self.thresholds
            .interface_sprawl_min_cluster
            .unwrap_or(match self.strictness {
                Strictness::Relaxed => 4,
                Strictness::Default => 3,
                Strictness::Strict => 2,
            })
    }

    pub fn effective_interface_sprawl_min_bundle_fields(&self) -> usize {
        self.thresholds
            .interface_sprawl_min_bundle_fields
            .unwrap_or(match self.strictness {
                Strictness::Relaxed => 4,
                Strictness::Default => 3,
                Strictness::Strict => 2,
            })
    }

    pub fn effective_interface_sprawl_min_overlap_ratio(&self) -> f32 {
        self.thresholds
            .interface_sprawl_min_overlap_ratio
            .unwrap_or(match self.strictness {
                Strictness::Relaxed => 0.55,
                Strictness::Default => 0.45,
                Strictness::Strict => 0.35,
            })
    }

    pub fn effective_interface_sprawl_max_findings(&self) -> usize {
        self.thresholds
            .interface_sprawl_max_findings
            .unwrap_or(60)
    }

    pub fn effective_lexicon_min_fields(&self) -> usize {
        self.thresholds
            .lexicon_min_fields
            .unwrap_or(match self.strictness {
                Strictness::Relaxed => 4,
                Strictness::Default => 3,
                Strictness::Strict => 2,
            })
    }

    pub fn effective_lexicon_max_smash_score(&self) -> usize {
        self.thresholds
            .lexicon_max_smash_score
            .unwrap_or(match self.strictness {
                Strictness::Relaxed => 4,
                Strictness::Default => 3,
                Strictness::Strict => 2,
            })
    }

    pub fn effective_lexicon_max_findings(&self) -> usize {
        self.thresholds.lexicon_max_findings.unwrap_or(40)
    }

    /// Embeddings are off by default until the model is cached locally (avoids surprise downloads).
    pub fn effective_embeddings_enabled(&self, project_root: &std::path::Path) -> bool {
        match self.thresholds.embeddings_enabled {
            Some(enabled) => enabled,
            None => crate::embeddings::embedding_model_ready_for(project_root),
        }
    }

    pub fn effective_redundant_logic_similarity(&self) -> f32 {
        self.thresholds
            .redundant_logic_similarity
            .unwrap_or(match self.strictness {
                Strictness::Relaxed => 0.90,
                Strictness::Default => 0.85,
                Strictness::Strict => 0.80,
            })
    }

    pub fn effective_god_file_max_exports(&self) -> usize {
        self.thresholds
            .god_file_max_exports
            .unwrap_or(match self.strictness {
                Strictness::Relaxed => 30,
                Strictness::Default => 20,
                Strictness::Strict => 15,
            })
    }

    pub fn effective_god_file_max_lines(&self) -> usize {
        self.thresholds
            .god_file_max_lines
            .unwrap_or(match self.strictness {
                Strictness::Relaxed => 1000,
                Strictness::Default => 500,
                Strictness::Strict => 300,
            })
    }

    pub fn effective_barrel_bloat_threshold(&self) -> usize {
        self.thresholds
            .barrel_bloat_threshold
            .unwrap_or(match self.strictness {
                Strictness::Relaxed => 30,
                Strictness::Default => 20,
                Strictness::Strict => 10,
            })
    }

    pub fn effective_dead_exports_min_per_file(&self) -> usize {
        self.thresholds.dead_exports_min_per_file.unwrap_or(1).max(1)
    }

    /// Whether a path is a test file (indexed for references only, never reported).
    pub fn is_test_path(&self, rel_path: &str) -> bool {
        self.entries
            .test_files
            .iter()
            .any(|pattern| glob_match(pattern, rel_path))
    }

    /// The first boundary whose paths match.
    pub fn boundary_of(&self, rel_path: &str) -> Option<&Boundary> {
        self.boundaries.iter().find(|boundary| {
            boundary
                .paths
                .iter()
                .any(|pattern| glob_match(pattern, rel_path))
        })
    }

    /// Split members of a duplicate group (by their file paths) into groups that may be
    /// reported together. Members on different sides of a fenced boundary never are.
    /// Returns (boundary name, member indexes).
    pub fn partition_by_boundary(&self, paths: &[&str]) -> Vec<(Option<String>, Vec<usize>)> {
        let names: Vec<Option<&Boundary>> = paths.iter().map(|path| self.boundary_of(path)).collect();
        let distinct: std::collections::BTreeSet<Option<&str>> = names
            .iter()
            .map(|boundary| boundary.map(|b| b.name.as_str()))
            .collect();
        let fenced = names.iter().any(|boundary| boundary.is_some_and(|b| b.fenced));
        if distinct.len() <= 1 || !fenced {
            let label = if distinct.len() == 1 {
                distinct.into_iter().next().flatten().map(String::from)
            } else {
                None
            };
            return vec![(label, (0..paths.len()).collect())];
        }
        distinct
            .into_iter()
            .map(|name| {
                let members = names
                    .iter()
                    .enumerate()
                    .filter(|(_, boundary)| boundary.map(|b| b.name.as_str()) == name)
                    .map(|(index, _)| index)
                    .collect();
                (name.map(String::from), members)
            })
            .collect()
    }

    /// Check if a pattern is enabled
    pub fn is_pattern_enabled(&self, pattern_name: &str) -> bool {
        !self.disabled_patterns.iter().any(|p| p == pattern_name)
    }

    /// Get the config file path for a project
    pub fn config_path(project_root: &Path) -> PathBuf {
        project_root.join(".prunerc")
    }

    /// Whether a relative path matches any configured exclude glob.
    pub fn excludes_path(&self, rel_path: &str) -> bool {
        self.exclude.iter().any(|pattern| glob_match(pattern, rel_path))
    }

    /// Whether an exclude glob is already configured (exact or broader).
    pub fn covers_glob(&self, glob: &str) -> bool {
        self.exclude.iter().any(|pattern| {
            pattern == glob || pattern_covers(pattern, glob)
        })
    }
}

/// Simple glob matching for exclude patterns.
pub fn glob_match(pattern: &str, path: &str) -> bool {
    if pattern.starts_with("*.") && !pattern.contains('/') {
        return path.ends_with(&pattern[1..]);
    }

    if pattern.ends_with("/**") && !pattern.starts_with("**/") {
        let prefix = pattern.trim_end_matches("/**");
        return path == prefix
            || path.starts_with(&format!("{}/", prefix))
            || path.starts_with(prefix);
    }

    if pattern.starts_with("**/") && pattern.ends_with("/**") {
        let segment = pattern
            .trim_start_matches("**/")
            .trim_end_matches("/**");
        let infix = format!("/{}/", segment);
        return path.contains(&infix)
            || path.starts_with(&format!("{}/", segment))
            || path.ends_with(&format!("/{}", segment))
            || path == segment;
    }

    if pattern.starts_with("**/") {
        let suffix = pattern.trim_start_matches("**/");
        if suffix.starts_with("*.") {
            return path.ends_with(&suffix[1..]);
        }
        return path.ends_with(suffix) || path.contains(suffix);
    }

    if pattern.contains("**") {
        let parts: Vec<_> = pattern.split("**").collect();
        if parts.len() == 2 {
            let prefix = parts[0].trim_end_matches('/');
            let suffix = parts[1].trim_start_matches('/');

            let matches_prefix = prefix.is_empty() || path.starts_with(prefix);
            let matches_suffix = if suffix.is_empty() {
                true
            } else if suffix.starts_with("*.") {
                path.ends_with(&suffix[1..])
            } else {
                path.ends_with(suffix) || path.contains(suffix)
            };

            return matches_prefix && matches_suffix;
        }
    }

    path == pattern || path.contains(pattern)
}

fn pattern_covers(existing: &str, candidate: &str) -> bool {
    if existing == candidate {
        return true;
    }
    if existing.ends_with("/**") {
        let prefix = existing.trim_end_matches("/**");
        return candidate.starts_with(prefix) || candidate.contains(prefix);
    }
    false
}

fn default_true() -> bool {
    true
}

fn default_entry_files() -> Vec<String> {
    ["vercel.ts", "vercel.js", "vercel.mjs"]
        .into_iter()
        .map(String::from)
        .collect()
}

fn default_test_files() -> Vec<String> {
    [
        "**/*.test.ts",
        "**/*.test.tsx",
        "**/*.test.js",
        "**/*.test.jsx",
        "**/*.test.mts",
        "**/*.spec.ts",
        "**/*.spec.tsx",
        "**/*.spec.js",
        "**/*.spec.jsx",
        "**/__tests__/**",
        "**/__mocks__/**",
        "e2e/**",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

fn default_next_app_entries() -> Vec<String> {
    [
        "page",
        "layout",
        "route",
        "loading",
        "error",
        "global-error",
        "global-not-found",
        "not-found",
        "forbidden",
        "unauthorized",
        "template",
        "default",
        "opengraph-image",
        "twitter-image",
        "icon",
        "apple-icon",
        "sitemap",
        "robots",
        "manifest",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

fn default_next_root_entries() -> Vec<String> {
    [
        "middleware",
        "proxy",
        "instrumentation",
        "instrumentation-client",
        "mdx-components",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

fn default_convex_builders() -> Vec<String> {
    [
        "query",
        "mutation",
        "action",
        "internalQuery",
        "internalMutation",
        "internalAction",
        "httpAction",
    ]
    .into_iter()
    .map(String::from)
    .collect()
}

fn default_convex_builder_suffixes() -> Vec<String> {
    ["Query", "Mutation", "Action"]
        .into_iter()
        .map(String::from)
        .collect()
}

fn default_convex_entry_modules() -> Vec<String> {
    ["schema", "http", "crons", "convex.config", "auth.config"]
        .into_iter()
        .map(String::from)
        .collect()
}

fn default_exclude() -> Vec<String> {
    vec![
        "node_modules/**".to_string(),
        "dist/**".to_string(),
        "build/**".to_string(),
        ".git/**".to_string(),
        ".prune/**".to_string(),
        "*.min.js".to_string(),
        "*.bundle.js".to_string(),
        "patches/**".to_string(),
        "**/_generated/**".to_string(),
        "**/*.test.ts".to_string(),
        "**/*.test.tsx".to_string(),
        "**/*.integration.test.ts".to_string(),
        "**/*.spec.ts".to_string(),
        "**/*.spec.tsx".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strictness_cycle() {
        assert_eq!(Strictness::Relaxed.cycle(), Strictness::Default);
        assert_eq!(Strictness::Default.cycle(), Strictness::Strict);
        assert_eq!(Strictness::Strict.cycle(), Strictness::Relaxed);
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.strictness, Strictness::Default);
        assert!(!config.exclude.is_empty());
        assert!(config.exclude.iter().any(|pattern| pattern == ".prune/**"));
    }

    #[test]
    fn omitted_exclude_keeps_defaults() {
        let config: Config = toml::from_str("strictness = \"strict\"\n").unwrap();
        assert!(config.exclude.iter().any(|pattern| pattern == ".prune/**"));
    }

    #[test]
    fn fenced_boundaries_split_duplicate_groups() {
        let config: Config = toml::from_str(
            r#"
[[boundaries]]
name = "inbox"
paths = ["packages/inbox/**"]
fenced = true

[[boundaries]]
name = "app"
paths = ["components/**", "convex/**"]
"#,
        )
        .unwrap();
        let groups = config.partition_by_boundary(&[
            "packages/inbox/a.ts",
            "packages/inbox/b.ts",
            "components/c.tsx",
            "convex/d.ts",
        ]);
        assert_eq!(groups.len(), 2);
        assert!(groups.contains(&(Some("inbox".into()), vec![0, 1])));
        assert!(groups.contains(&(Some("app".into()), vec![2, 3])));
        // No fence involved: one group.
        let groups = config.partition_by_boundary(&["components/c.tsx", "convex/d.ts", "lib/e.ts"]);
        assert_eq!(groups, vec![(None, vec![0, 1, 2])]);
    }

    #[test]
    fn glob_match_generated_and_tests() {
        assert!(glob_match("**/_generated/**", "convex/_generated/api.d.ts"));
        assert!(glob_match("**/_generated/**", "convex/components/analytics/_generated/api.ts"));
        assert!(!glob_match("**/_generated/**", "packages/domain/src/event/discovery/generated-locations.ts"));
        assert!(glob_match("**/*.test.ts", "src/foo/bar.integration.test.ts"));
        assert!(glob_match("patches/**", "patches/foo.patch"));
    }
}
