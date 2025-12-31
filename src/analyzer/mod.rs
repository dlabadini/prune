//! Codebase analyzer using tree-sitter for parsing

mod parser;
mod indexer;

pub use parser::Parser;
pub use indexer::Indexer;

use crate::config::Config;
use crate::db::Database;
use crate::patterns::{Finding, run_all_detectors};
use std::path::{Path, PathBuf};
use thiserror::Error;
use ignore::WalkBuilder;
use std::time::Instant;

#[derive(Error, Debug)]
pub enum AnalyzerError {
    #[error("Failed to access path: {0}")]
    PathError(String),
    #[error("Database error: {0}")]
    DbError(#[from] crate::db::DbError),
    #[error("Parse error: {0}")]
    ParseError(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

/// Result of a codebase scan
#[derive(Debug, Clone)]
pub struct ScanResult {
    pub files_scanned: usize,
    pub files_changed: usize,
    pub total_lines: usize,
    pub symbols_found: usize,
    pub findings: Vec<Finding>,
    pub scan_duration_ms: u64,
}

/// Main analyzer for scanning codebases
pub struct Analyzer {
    root: PathBuf,
    config: Config,
    db: Database,
    indexer: Indexer,
}

impl Analyzer {
    /// Create a new analyzer for a project root
    pub fn new(root: &Path) -> Result<Self, AnalyzerError> {
        let config = Config::find_and_load(root).unwrap_or_default();
        let db = Database::open_for_project(root)?;
        let indexer = Indexer::new();

        Ok(Self {
            root: root.to_path_buf(),
            config,
            db,
            indexer,
        })
    }

    /// Create analyzer with custom config
    pub fn with_config(root: &Path, config: Config) -> Result<Self, AnalyzerError> {
        let db = Database::open_for_project(root)?;
        let indexer = Indexer::new();

        Ok(Self {
            root: root.to_path_buf(),
            config,
            db,
            indexer,
        })
    }

    /// Get the current configuration
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Get mutable reference to configuration
    pub fn config_mut(&mut self) -> &mut Config {
        &mut self.config
    }

    /// Scan the codebase and detect patterns
    pub fn scan(&mut self) -> Result<ScanResult, AnalyzerError> {
        let start = Instant::now();

        let files = self.collect_files()?;
        let mut files_scanned = 0;
        let mut files_changed = 0;

        for file_path in &files {
            files_scanned += 1;

            // Check if file has changed
            let content = std::fs::read_to_string(file_path)?;
            let hash = calculate_hash(&content);
            let rel_path = self.relative_path(file_path);

            let needs_reindex = match self.db.get_file_hash(&rel_path)? {
                Some(existing_hash) => existing_hash != hash,
                None => true,
            };

            if needs_reindex {
                files_changed += 1;
                self.index_file(file_path, &content, &hash)?;
            }
        }

        // Run pattern detection
        let findings = run_all_detectors(&self.db, &self.config);

        let stats = self.db.get_stats()?;

        Ok(ScanResult {
            files_scanned,
            files_changed,
            total_lines: stats.total_lines,
            symbols_found: stats.symbol_count,
            findings,
            scan_duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Re-index a single file (for watch mode)
    pub fn reindex_file(&mut self, file_path: &Path) -> Result<(), AnalyzerError> {
        if !file_path.exists() {
            // File was deleted
            let rel_path = self.relative_path(file_path);
            self.db.delete_file(&rel_path)?;
            return Ok(());
        }

        let content = std::fs::read_to_string(file_path)?;
        let hash = calculate_hash(&content);

        self.index_file(file_path, &content, &hash)?;
        Ok(())
    }

    /// Get current findings without rescanning
    pub fn get_findings(&self) -> Vec<Finding> {
        run_all_detectors(&self.db, &self.config)
    }

    /// Get database statistics
    pub fn get_stats(&self) -> Result<crate::db::DbStats, AnalyzerError> {
        Ok(self.db.get_stats()?)
    }

    /// Collect all source files to analyze
    fn collect_files(&self) -> Result<Vec<PathBuf>, AnalyzerError> {
        let mut files = Vec::new();

        let walker = WalkBuilder::new(&self.root)
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

            // Check file extension
            let ext = path.extension()
                .and_then(|e| e.to_str())
                .unwrap_or("");

            if !is_supported_extension(ext) {
                continue;
            }

            // Check custom exclude patterns
            let rel_path = self.relative_path(path);
            if self.is_excluded(&rel_path) {
                continue;
            }

            files.push(path.to_path_buf());
        }

        Ok(files)
    }

    /// Index a single file
    fn index_file(
        &mut self,
        file_path: &Path,
        content: &str,
        hash: &str,
    ) -> Result<(), AnalyzerError> {
        let rel_path = self.relative_path(file_path);
        let line_count = content.lines().count();

        // Upsert file record
        let file_id = self.db.upsert_file(&rel_path, hash, line_count)?;

        // Clear existing symbols for this file
        self.db.clear_file_symbols(file_id)?;

        // Parse and extract symbols
        let ext = file_path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");

        let symbols = self.indexer.parse_file(content, ext)?;

        // Insert symbols
        for symbol in symbols {
            self.db.insert_symbol(
                file_id,
                &symbol.name,
                symbol.kind,
                symbol.start_line,
                symbol.end_line,
                symbol.is_exported,
                &symbol.source_text,
            )?;
        }

        // Parse and extract imports
        let imports = self.indexer.parse_imports(content, ext)?;

        for import in imports {
            // Try to resolve target file
            let target_file_id = if import.target_path.starts_with('.') {
                // Relative import - try to resolve
                let resolved = resolve_import(&self.root, &rel_path, &import.target_path);
                if let Some(resolved_path) = resolved {
                    self.db.get_file_id(&resolved_path)?
                } else {
                    None
                }
            } else {
                None
            };

            self.db.insert_import(
                file_id,
                target_file_id,
                &import.target_path,
                &import.imported_names,
                import.is_type_only,
            )?;
        }

        Ok(())
    }

    /// Get relative path from root
    fn relative_path(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string()
    }

    /// Check if path should be excluded
    fn is_excluded(&self, rel_path: &str) -> bool {
        // Check include patterns first (they override exclude)
        if !self.config.include.is_empty() {
            let included = self.config.include.iter()
                .any(|pattern| glob_match(pattern, rel_path));
            if !included {
                return true;
            }
        }

        // Check exclude patterns
        self.config.exclude.iter()
            .any(|pattern| glob_match(pattern, rel_path))
    }

    /// Get the project root
    pub fn root(&self) -> &Path {
        &self.root
    }
}

fn is_supported_extension(ext: &str) -> bool {
    matches!(ext, "ts" | "tsx" | "js" | "jsx" | "mts" | "mjs" | "cts" | "cjs")
}

fn calculate_hash(content: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn glob_match(pattern: &str, path: &str) -> bool {
    // Simple glob matching - could be improved with proper glob crate
    if pattern.contains("**") {
        let parts: Vec<_> = pattern.split("**").collect();
        if parts.len() == 2 {
            let prefix = parts[0].trim_end_matches('/');
            let suffix = parts[1].trim_start_matches('/');

            let matches_prefix = prefix.is_empty() || path.starts_with(prefix);
            let matches_suffix = if suffix.is_empty() {
                true
            } else if suffix.starts_with("*.") {
                // Handle **/*.ext patterns
                path.ends_with(&suffix[1..])
            } else {
                path.ends_with(suffix)
            };

            return matches_prefix && matches_suffix;
        }
    }

    if pattern.starts_with("*.") {
        let ext = &pattern[1..];
        return path.ends_with(ext);
    }

    path.contains(pattern)
}

fn resolve_import(root: &Path, source_file: &str, import_path: &str) -> Option<String> {
    let source_dir = Path::new(source_file).parent()?;
    let resolved = source_dir.join(import_path);

    // Try common extensions
    let extensions = ["ts", "tsx", "js", "jsx", "mts", "mjs"];

    for ext in extensions {
        let with_ext = resolved.with_extension(ext);
        let full_path = root.join(&with_ext);
        if full_path.exists() {
            return Some(with_ext.to_string_lossy().to_string());
        }
    }

    // Try index files
    for ext in extensions {
        let index_file = resolved.join(format!("index.{}", ext));
        let full_path = root.join(&index_file);
        if full_path.exists() {
            return Some(index_file.to_string_lossy().to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_match() {
        assert!(glob_match("**/*.ts", "src/foo/bar.ts"));
        assert!(glob_match("node_modules/**", "node_modules/foo/bar.js"));
        assert!(glob_match("*.min.js", "bundle.min.js"));
        assert!(!glob_match("*.ts", "foo.js"));
    }

    #[test]
    fn test_is_supported_extension() {
        assert!(is_supported_extension("ts"));
        assert!(is_supported_extension("tsx"));
        assert!(is_supported_extension("js"));
        assert!(!is_supported_extension("rs"));
        assert!(!is_supported_extension("py"));
    }
}
