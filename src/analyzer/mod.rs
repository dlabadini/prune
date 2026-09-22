//! Codebase analyzer using tree-sitter for parsing

mod indexer;
mod parser;

use oxc_resolver::{ResolveOptions, Resolver, TsconfigDiscovery};

pub use indexer::Indexer;

use crate::config::Config;
use crate::db::Database;
use crate::patterns::{run_all_detectors, Finding};
use ignore::WalkBuilder;
use std::path::{Path, PathBuf};
use std::time::Instant;
use thiserror::Error;

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
    pub score: crate::score::RepoScore,
    pub scan_duration_ms: u64,
}

/// What the scanner is doing, so a UI can name the step.
#[derive(Debug, Clone)]
pub enum ScanProgress {
    Indexing {
        done: usize,
        total: usize,
        file: String,
    },
    Scoring,
}

/// Main analyzer for scanning codebases
pub struct Analyzer {
    root: PathBuf,
    config: Config,
    db: Database,
    indexer: Indexer,
    resolver: Resolver,
}

impl Analyzer {
    /// Create a new analyzer for a project root
    pub fn new(root: &Path) -> Result<Self, AnalyzerError> {
        let config = Config::find_and_load(root).unwrap_or_default();
        crate::embeddings::prepare_cache(root);
        let db = Database::open_for_project(root)?;
        let indexer = Indexer::new();
        let resolver = project_resolver();

        Ok(Self {
            root: root.to_path_buf(),
            config,
            db,
            indexer,
            resolver,
        })
    }

    /// Create analyzer with custom config
    pub fn with_config(root: &Path, config: Config) -> Result<Self, AnalyzerError> {
        crate::embeddings::prepare_cache(root);
        let db = Database::open_for_project(root)?;
        let indexer = Indexer::new();
        let resolver = project_resolver();

        Ok(Self {
            root: root.to_path_buf(),
            config,
            db,
            indexer,
            resolver,
        })
    }

    /// Project root being analyzed.
    pub fn root(&self) -> &Path {
        &self.root
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
        self.scan_reporting(&mut |_| {})
    }

    /// Scan, reporting each step to `report`.
    pub fn scan_reporting(
        &mut self,
        report: &mut dyn FnMut(ScanProgress),
    ) -> Result<ScanResult, AnalyzerError> {
        let start = Instant::now();

        let files = self.collect_files()?;
        self.purge_stale_indexed_files(&files)?;
        let mut files_scanned = 0;
        let mut files_changed = 0;
        let total = files.len();

        for file_path in &files {
            files_scanned += 1;
            report(ScanProgress::Indexing {
                done: files_scanned,
                total,
                file: self.relative_path(file_path),
            });

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

        self.finalize_index(report)?;
        self.sync_embeddings()?;

        report(ScanProgress::Scoring);
        let mut findings = run_all_detectors(&self.db, &self.config);
        crate::gitinfo::annotate_findings(&self.root, &mut findings);
        let score = crate::score::score_repo(&self.db, &self.config, &findings)?;

        let stats = self.db.get_stats()?;

        Ok(ScanResult {
            files_scanned,
            files_changed,
            total_lines: stats.total_lines,
            symbols_found: stats.symbol_count,
            findings,
            score,
            scan_duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Run detectors on the current index without re-walking files.
    pub fn analyze(&mut self) -> Result<ScanResult, AnalyzerError> {
        let start = Instant::now();
        self.sync_embeddings()?;
        let mut findings = run_all_detectors(&self.db, &self.config);
        crate::gitinfo::annotate_findings(&self.root, &mut findings);
        let score = crate::score::score_repo(&self.db, &self.config, &findings)?;
        let stats = self.db.get_stats()?;
        Ok(ScanResult {
            files_scanned: stats.file_count,
            files_changed: 0,
            total_lines: stats.total_lines,
            symbols_found: stats.symbol_count,
            findings,
            score,
            scan_duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Score an existing finding list against the current index.
    pub fn score_findings(
        &self,
        findings: &[Finding],
    ) -> Result<crate::score::RepoScore, AnalyzerError> {
        Ok(crate::score::score_repo(&self.db, &self.config, findings)?)
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
        self.finalize_index(&mut |_| {})?;
        self.sync_embeddings()?;
        Ok(())
    }

    /// Embed new/changed symbols incrementally for semantic duplicate detection.
    fn sync_embeddings(&mut self) -> Result<(), AnalyzerError> {
        if !self.config.effective_embeddings_enabled(&self.root) {
            return Ok(());
        }
        let symbols = self.db.symbols_for_embedding()?;
        if symbols.is_empty() {
            return Ok(());
        }

        let mut pending: Vec<(i64, String, String)> = Vec::new();
        for (symbol_id, source_text) in symbols {
            let hash = crate::embeddings::content_key(&source_text);
            match self.db.cached_embedding(&hash)? {
                Some(vector) => {
                    self.db
                        .store_embedding(symbol_id, &hash, &vector)
                        .map_err(AnalyzerError::DbError)?;
                }
                None => pending.push((symbol_id, hash, source_text)),
            }
        }

        const BATCH: usize = 48;
        for chunk in pending.chunks(BATCH) {
            let texts = chunk
                .iter()
                .map(|(_, _, text)| text.clone())
                .collect::<Vec<_>>();
            let Some(vectors) = crate::embeddings::embed_texts(&texts) else {
                tracing::debug!("skipping embedding batch: model unavailable");
                return Ok(());
            };
            for ((symbol_id, hash, _), vector) in chunk.iter().zip(vectors) {
                self.db
                    .store_embedding(*symbol_id, hash, &vector)
                    .map_err(AnalyzerError::DbError)?;
            }
        }
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

    /// Drop indexed files that are no longer scanned (excluded, deleted, or renamed).
    fn purge_stale_indexed_files(&self, files: &[PathBuf]) -> Result<(), AnalyzerError> {
        let active: std::collections::HashSet<String> =
            files.iter().map(|path| self.relative_path(path)).collect();
        for file in self.db.get_all_files()? {
            if !active.contains(&file.path) {
                self.db.delete_file(&file.path)?;
            }
        }
        Ok(())
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
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

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
        let ext = file_path.extension().and_then(|e| e.to_str()).unwrap_or("");

        let indexed = self.indexer.index_source(content, ext)?;

        for symbol in indexed.parsed.symbols {
            self.db.insert_symbol(
                file_id,
                &symbol.name,
                symbol.kind,
                symbol.start_line,
                symbol.end_line,
                symbol.is_exported,
                symbol.is_default,
                &symbol.source_text,
            )?;
        }

        for import in indexed.parsed.imports {
            self.db.insert_import(
                file_id,
                None,
                &import.target_path,
                &import.imported_names,
                import.is_type_only,
                import.is_reexport,
                import.import_all,
                import.line,
            )?;
        }

        for hit in indexed.structural {
            self.db
                .insert_structural_note(file_id, hit.kind, &hit.name, hit.line, &hit.detail)?;
        }

        for use_site in indexed.parsed.local_uses {
            if let Some(symbol_id) =
                self.db
                    .find_symbol_at(file_id, &use_site.name, use_site.decl_line)?
            {
                self.db
                    .insert_reference(symbol_id, file_id, use_site.use_line)?;
            }
        }

        Ok(())
    }

    /// Resolve imports and record cross-file references.
    fn finalize_index(
        &mut self,
        _report: &mut dyn FnMut(ScanProgress),
    ) -> Result<(), AnalyzerError> {
        self.db.clear_refs()?;
        let edges = self.db.import_edges()?;
        for edge in edges {
            let absolute = self.root.join(&edge.source_path);
            let full = match self.resolver.resolve_file(&absolute, &edge.specifier) {
                Ok(resolved) => resolved.full_path(),
                Err(_) => match resolve_relative_import(&absolute, &edge.specifier) {
                    Some(path) => path,
                    None => continue,
                },
            };
            let root = std::fs::canonicalize(&self.root).unwrap_or_else(|_| self.root.clone());
            let full = std::fs::canonicalize(&full).unwrap_or(full);
            let Some(relative) = full
                .strip_prefix(&root)
                .ok()
                .map(|path| path.to_string_lossy().replace('\\', "/"))
            else {
                continue;
            };
            let Some(target_id) = self.db.get_file_id(&relative)? else {
                continue;
            };
            self.db.set_import_target(edge.id, target_id)?;

            let symbol_ids = if edge.import_all {
                self.db.exported_symbol_ids(target_id)?
            } else {
                let mut ids = Vec::new();
                for name in &edge.imported_names {
                    if let Some(id) = self.db.find_export(target_id, name)? {
                        ids.push(id);
                    }
                }
                ids
            };
            for symbol_id in symbol_ids {
                self.db
                    .insert_reference(symbol_id, edge.source_file_id, edge.line)?;
            }
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
            let included = self
                .config
                .include
                .iter()
                .any(|pattern| crate::config::glob_match(pattern, rel_path));
            if !included {
                return true;
            }
        }

        // Check exclude patterns
        self.config.excludes_path(rel_path)
    }

}

fn is_supported_extension(ext: &str) -> bool {
    matches!(
        ext,
        "ts" | "tsx" | "js" | "jsx" | "mts" | "mjs" | "cts" | "cjs"
    )
}

fn calculate_hash(content: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn resolve_relative_import(source: &Path, specifier: &str) -> Option<PathBuf> {
    if !(specifier.starts_with("./") || specifier.starts_with("../")) {
        return None;
    }
    let base = source.parent()?;
    let joined = base.join(specifier);
    if joined.is_file() {
        return Some(joined);
    }
    if joined.extension().is_none() {
        for ext in ["ts", "tsx", "js", "jsx", "mjs", "mts"] {
            let candidate = joined.with_extension(ext);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn project_resolver() -> Resolver {
    Resolver::new(ResolveOptions {
        extensions: vec![
            ".ts".into(),
            ".tsx".into(),
            ".js".into(),
            ".jsx".into(),
            ".mjs".into(),
            ".cjs".into(),
            ".mts".into(),
            ".cts".into(),
        ],
        extension_alias: vec![(
            ".js".into(),
            vec![".ts".into(), ".tsx".into(), ".js".into(), ".jsx".into()],
        )],
        condition_names: vec![
            "types".into(),
            "import".into(),
            "require".into(),
            "default".into(),
        ],
        main_fields: vec!["types".into(), "module".into(), "main".into()],
        tsconfig: Some(TsconfigDiscovery::Auto),
        ..ResolveOptions::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_match() {
        assert!(crate::config::glob_match("**/*.ts", "src/foo/bar.ts"));
        assert!(crate::config::glob_match("node_modules/**", "node_modules/foo/bar.js"));
        assert!(crate::config::glob_match("**/_generated/**", "convex/_generated/server.d.ts"));
        assert!(crate::config::glob_match("*.min.js", "bundle.min.js"));
        assert!(!crate::config::glob_match("*.ts", "foo.js"));
    }

    #[test]
    fn test_is_supported_extension() {
        assert!(is_supported_extension("ts"));
        assert!(is_supported_extension("tsx"));
        assert!(is_supported_extension("js"));
        assert!(!is_supported_extension("rs"));
        assert!(!is_supported_extension("py"));
    }

    #[test]
    fn scan_resolves_imports_cycles_and_dead_exports() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"name":"prune-fixture","type":"module"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("tsconfig.json"),
            r#"{"compilerOptions":{"moduleResolution":"bundler","paths":{}}}"#,
        )
        .unwrap();
        std::fs::write(
            src.join("a.ts"),
            r#"
import { fromB } from "./b";
export function fromA() { return fromB(); }
export function unused1() { return 1; }
export function unused2() { return 2; }
export function unused3() { return 3; }
export function unused4() { return 4; }
export function unused5() { return 5; }
"#,
        )
        .unwrap();
        std::fs::write(
            src.join("b.ts"),
            r#"
import { fromA } from "./a";
export function fromB() { return fromA(); }
export function pass(value: string) { return trim(value); }
"#,
        )
        .unwrap();

        let mut analyzer = Analyzer::new(dir.path()).unwrap();
        let result = analyzer.scan().unwrap();
        assert!(result.symbols_found >= 4);
        assert!(result
            .findings
            .iter()
            .any(|finding| finding.pattern_type == crate::patterns::PatternType::ImportCycle));
        let kinds: Vec<_> = result
            .findings
            .iter()
            .map(|finding| finding.pattern_type.as_str())
            .collect();
        assert!(
            kinds.iter().any(|kind| *kind == "dead_exports"),
            "findings={kinds:?}"
        );
        assert!(
            kinds.iter().any(|kind| *kind == "over_abstraction"),
            "findings={kinds:?}"
        );
        let again = analyzer.scan().unwrap();
        let first_ids: Vec<_> = result
            .findings
            .iter()
            .map(|finding| finding.id.clone())
            .collect();
        let second_ids: Vec<_> = again
            .findings
            .iter()
            .map(|finding| finding.id.clone())
            .collect();
        assert_eq!(first_ids, second_ids);
        assert!(again.score.total <= 105.0);
    }
}
