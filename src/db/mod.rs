//! Database storage for Prune using SQLite

use crate::analyzer::parser::{ImportBinding, ImportKind, NameRefKind};
use crate::embeddings::{self, EMBED_DIM};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Once;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DbError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("Database not initialized")]
    NotInitialized,
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Represents a source file in the codebase
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceFile {
    pub id: i64,
    pub path: String,
    pub hash: String,
    pub line_count: usize,
    pub last_indexed: DateTime<Utc>,
}

/// Represents a symbol (function, class, interface, type, variable)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub id: i64,
    pub file_id: i64,
    pub name: String,
    pub kind: SymbolKind,
    pub start_line: usize,
    pub end_line: usize,
    pub is_exported: bool,
    pub is_default: bool,
    pub source_text: String,
    /// Callee of the initializer (`query`, `v.object`), when the symbol is `const x = f(...)`.
    #[serde(default)]
    pub callee: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    Class,
    Interface,
    TypeAlias,
    Variable,
    Enum,
    Namespace,
}

impl SymbolKind {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "function" => Some(SymbolKind::Function),
            "class" => Some(SymbolKind::Class),
            "interface" => Some(SymbolKind::Interface),
            "type_alias" => Some(SymbolKind::TypeAlias),
            "variable" => Some(SymbolKind::Variable),
            "enum" => Some(SymbolKind::Enum),
            "namespace" => Some(SymbolKind::Namespace),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            SymbolKind::Function => "function",
            SymbolKind::Class => "class",
            SymbolKind::Interface => "interface",
            SymbolKind::TypeAlias => "type_alias",
            SymbolKind::Variable => "variable",
            SymbolKind::Enum => "enum",
            SymbolKind::Namespace => "namespace",
        }
    }
}

/// Bump when the parser or schema changes what is stored; the index is rebuilt.
pub const INDEX_VERSION: i64 = 3;

/// Whether a file is product code or only read for references (tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileRole {
    Source,
    Test,
}

impl FileRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileRole::Source => "source",
            FileRole::Test => "test",
        }
    }

    pub fn parse(value: &str) -> Self {
        if value == "test" {
            FileRole::Test
        } else {
            FileRole::Source
        }
    }
}

/// A module edge as stored in the index (see `analyzer::parser::ParsedImport`).
#[derive(Debug, Clone)]
pub struct ImportEdge {
    pub id: i64,
    pub source_file_id: i64,
    pub source_path: String,
    pub target_file_id: Option<i64>,
    pub target_path: Option<String>,
    pub specifier: String,
    pub kind: ImportKind,
    pub bindings: Vec<ImportBinding>,
    pub namespace: bool,
    pub star: bool,
    pub namespace_members: Option<Vec<String>>,
    pub is_type_only: bool,
    pub line: usize,
}

impl ImportEdge {
    pub fn imported_names(&self) -> Vec<String> {
        self.bindings.iter().map(|b| b.imported.clone()).collect()
    }
}

/// A non-import reference stored for framework adapters.
#[derive(Debug, Clone)]
pub struct StoredNameRef {
    pub file_id: i64,
    pub kind: NameRefKind,
    pub value: String,
    pub specifier: Option<String>,
    pub line: usize,
}

/// One resolved use of a symbol from a file.
#[derive(Debug, Clone, Copy)]
pub struct RefRow {
    pub symbol_id: i64,
    pub file_id: i64,
    pub line: usize,
    /// Resolved across modules (import, framework reference) rather than a same-scope use.
    pub derived: bool,
}

/// File row with its role.
#[derive(Debug, Clone)]
pub struct FileRow {
    pub id: i64,
    pub path: String,
    pub role: FileRole,
    pub line_count: usize,
}

/// An ast-grep hit stored with the file it came from.
#[derive(Debug, Clone)]
pub struct StructuralNote {
    pub file_id: i64,
    pub file_path: String,
    pub kind: String,
    pub name: String,
    pub line: usize,
    pub detail: String,
}

/// Represents a reference to a symbol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reference {
    pub id: i64,
    pub symbol_id: i64,
    pub file_id: i64,
    pub line: usize,
}

/// Database handle for Prune
pub struct Database {
    conn: Connection,
    #[allow(dead_code)]
    path: PathBuf,
}

impl Database {
    /// Open or create a database at the specified path
    pub fn open(path: &Path) -> Result<Self, DbError> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        ensure_sqlite_vec();
        let conn = Connection::open(path)?;
        // Enable foreign keys - must be done before any operations
        conn.execute_batch("PRAGMA foreign_keys = OFF;")?;
        let db = Self {
            conn,
            path: path.to_path_buf(),
        };
        db.init_schema()?;
        Ok(db)
    }

    /// Open database in the .prune directory of a project
    pub fn open_for_project(project_root: &Path) -> Result<Self, DbError> {
        let prune_dir = crate::workspace::ensure_data_dir(project_root)?;
        let db_path = prune_dir.join("index.db");
        Self::open(&db_path)
    }

    /// Initialize the database schema
    fn init_schema(&self) -> Result<(), DbError> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version != INDEX_VERSION {
            // The index is a cache: rebuild it when its shape changes. Embeddings are keyed by
            // content hash and survive.
            self.conn.execute_batch(
                r#"
                DROP TABLE IF EXISTS refs;
                DROP TABLE IF EXISTS imports;
                DROP TABLE IF EXISTS symbols;
                DROP TABLE IF EXISTS structural_notes;
                DROP TABLE IF EXISTS name_refs;
                DROP TABLE IF EXISTS entries;
                DROP TABLE IF EXISTS symbol_roots;
                DROP TABLE IF EXISTS symbol_tags;
                DROP TABLE IF EXISTS findings;
                DROP TABLE IF EXISTS files;
                "#,
            )?;
            let _ = self.conn.execute("DROP TABLE IF EXISTS vec_symbols", []);
        }
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY,
                path TEXT UNIQUE NOT NULL,
                hash TEXT NOT NULL,
                line_count INTEGER NOT NULL,
                last_indexed TEXT NOT NULL,
                role TEXT NOT NULL DEFAULT 'source'
            );

            CREATE TABLE IF NOT EXISTS symbols (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                is_exported INTEGER NOT NULL DEFAULT 0,
                is_default INTEGER NOT NULL DEFAULT 0,
                source_text TEXT NOT NULL,
                callee TEXT,
                FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS imports (
                id INTEGER PRIMARY KEY,
                source_file_id INTEGER NOT NULL,
                target_file_id INTEGER,
                target_path TEXT NOT NULL,
                kind TEXT NOT NULL DEFAULT 'static',
                bindings TEXT NOT NULL DEFAULT '[]',
                namespace INTEGER NOT NULL DEFAULT 0,
                star INTEGER NOT NULL DEFAULT 0,
                namespace_members TEXT,
                is_type_only INTEGER NOT NULL DEFAULT 0,
                line INTEGER NOT NULL DEFAULT 1,
                FOREIGN KEY (source_file_id) REFERENCES files(id) ON DELETE CASCADE,
                FOREIGN KEY (target_file_id) REFERENCES files(id) ON DELETE SET NULL
            );

            CREATE TABLE IF NOT EXISTS name_refs (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                kind TEXT NOT NULL,
                value TEXT NOT NULL,
                specifier TEXT,
                line INTEGER NOT NULL,
                FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS entries (
                file_id INTEGER NOT NULL,
                reason TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS symbol_roots (
                symbol_id INTEGER NOT NULL,
                reason TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS symbol_tags (
                symbol_id INTEGER NOT NULL,
                tag TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS structural_notes (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                kind TEXT NOT NULL,
                name TEXT NOT NULL,
                line INTEGER NOT NULL,
                detail TEXT NOT NULL,
                FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS embed_cache (
                content_hash TEXT PRIMARY KEY,
                embedding BLOB NOT NULL
            );

            CREATE TABLE IF NOT EXISTS refs (
                id INTEGER PRIMARY KEY,
                symbol_id INTEGER NOT NULL,
                file_id INTEGER NOT NULL,
                line INTEGER NOT NULL,
                derived INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (symbol_id) REFERENCES symbols(id) ON DELETE CASCADE,
                FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS findings (
                id TEXT PRIMARY KEY,
                pattern_type TEXT NOT NULL,
                severity TEXT NOT NULL,
                confidence REAL NOT NULL,
                title TEXT NOT NULL,
                description TEXT NOT NULL,
                locations TEXT NOT NULL,
                first_detected TEXT NOT NULL,
                last_seen TEXT NOT NULL,
                resolved INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file_id);
            CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
            CREATE INDEX IF NOT EXISTS idx_symbols_kind ON symbols(kind);
            CREATE INDEX IF NOT EXISTS idx_imports_source ON imports(source_file_id);
            CREATE INDEX IF NOT EXISTS idx_imports_target ON imports(target_file_id);
            CREATE INDEX IF NOT EXISTS idx_name_refs_file ON name_refs(file_id);
            CREATE INDEX IF NOT EXISTS idx_refs_symbol ON refs(symbol_id);
            CREATE INDEX IF NOT EXISTS idx_refs_file ON refs(file_id);
            CREATE INDEX IF NOT EXISTS idx_findings_pattern ON findings(pattern_type);
            CREATE INDEX IF NOT EXISTS idx_findings_resolved ON findings(resolved);
            "#,
        )?;
        self.conn
            .execute_batch(&format!("PRAGMA user_version = {INDEX_VERSION};"))?;
        self.ensure_vector_table()?;
        Ok(())
    }

    /// Run `work` in one transaction (bulk index writes are much faster this way).
    pub fn transaction<T>(
        &self,
        work: impl FnOnce(&Self) -> Result<T, DbError>,
    ) -> Result<T, DbError> {
        self.conn.execute_batch("BEGIN")?;
        match work(self) {
            Ok(value) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(value)
            }
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    fn ensure_vector_table(&self) -> Result<(), DbError> {
        let sql = format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS vec_symbols USING vec0(embedding float[{EMBED_DIM}])"
        );
        if let Err(error) = self.conn.execute(&sql, []) {
            tracing::warn!("sqlite-vec index unavailable: {error}");
        }
        Ok(())
    }

    /// Insert or update a file record
    pub fn upsert_file(&self, path: &str, hash: &str, line_count: usize) -> Result<i64, DbError> {
        self.upsert_file_with_role(path, hash, line_count, FileRole::Source)
    }

    /// Insert or update a file record with its role.
    pub fn upsert_file_with_role(
        &self,
        path: &str,
        hash: &str,
        line_count: usize,
        role: FileRole,
    ) -> Result<i64, DbError> {
        let now = Utc::now().to_rfc3339();

        self.conn.execute(
            r#"
            INSERT INTO files (path, hash, line_count, last_indexed, role)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(path) DO UPDATE SET
                hash = excluded.hash,
                line_count = excluded.line_count,
                last_indexed = excluded.last_indexed,
                role = excluded.role
            "#,
            params![path, hash, line_count, now, role.as_str()],
        )?;

        let id = self.conn.query_row(
            "SELECT id FROM files WHERE path = ?1",
            params![path],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    /// Get file ID by path
    pub fn get_file_id(&self, path: &str) -> Result<Option<i64>, DbError> {
        let mut stmt = self.conn.prepare("SELECT id FROM files WHERE path = ?1")?;
        let result = stmt.query_row(params![path], |row| row.get(0));
        match result {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get file hash by path (for change detection)
    pub fn get_file_hash(&self, path: &str) -> Result<Option<String>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT hash FROM files WHERE path = ?1")?;
        let result = stmt.query_row(params![path], |row| row.get(0));
        match result {
            Ok(hash) => Ok(Some(hash)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Delete file and all related data
    pub fn delete_file(&self, path: &str) -> Result<(), DbError> {
        self.conn
            .execute("DELETE FROM files WHERE path = ?1", params![path])?;
        Ok(())
    }

    /// Clear all symbols for a file (before re-indexing)
    pub fn clear_file_symbols(&self, file_id: i64) -> Result<(), DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM symbols WHERE file_id = ?1")?;
        let ids = stmt
            .query_map(params![file_id], |row| row.get::<_, i64>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);
        for id in ids {
            let _ = self
                .conn
                .execute("DELETE FROM vec_symbols WHERE rowid = ?1", params![id]);
            self.conn
                .execute("DELETE FROM refs WHERE symbol_id = ?1", params![id])?;
        }
        self.conn
            .execute("DELETE FROM refs WHERE file_id = ?1", params![file_id])?;
        self.conn
            .execute("DELETE FROM symbols WHERE file_id = ?1", params![file_id])?;
        self.conn.execute(
            "DELETE FROM imports WHERE source_file_id = ?1",
            params![file_id],
        )?;
        self.conn.execute(
            "DELETE FROM structural_notes WHERE file_id = ?1",
            params![file_id],
        )?;
        self.conn
            .execute("DELETE FROM name_refs WHERE file_id = ?1", params![file_id])?;
        Ok(())
    }

    /// Drop everything `finalize` derives from the per-file index.
    pub fn clear_refs(&self) -> Result<(), DbError> {
        self.conn.execute_batch(
            "DELETE FROM refs WHERE derived = 1; DELETE FROM entries; DELETE FROM symbol_roots;
             DELETE FROM symbol_tags; UPDATE imports SET target_file_id = NULL;",
        )?;
        Ok(())
    }

    /// Insert a symbol
    #[allow(clippy::too_many_arguments)]
    pub fn insert_symbol(
        &self,
        file_id: i64,
        name: &str,
        kind: SymbolKind,
        start_line: usize,
        end_line: usize,
        is_exported: bool,
        is_default: bool,
        source_text: &str,
    ) -> Result<i64, DbError> {
        self.insert_symbol_row(
            file_id,
            &crate::analyzer::parser::ParsedSymbol {
                name: name.to_string(),
                kind,
                start_line,
                end_line,
                is_exported,
                is_default,
                source_text: source_text.to_string(),
                callee: None,
            },
        )
    }

    /// Insert a parsed symbol.
    pub fn insert_symbol_row(
        &self,
        file_id: i64,
        symbol: &crate::analyzer::parser::ParsedSymbol,
    ) -> Result<i64, DbError> {
        self.conn.execute(
            r#"
            INSERT INTO symbols (file_id, name, kind, start_line, end_line, is_exported, is_default, source_text, callee)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                file_id,
                symbol.name,
                symbol.kind.as_str(),
                symbol.start_line,
                symbol.end_line,
                symbol.is_exported,
                symbol.is_default,
                symbol.source_text,
                symbol.callee
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Record a use found inside the symbol's own file (kept across `finalize`).
    pub fn insert_reference(
        &self,
        symbol_id: i64,
        file_id: i64,
        line: usize,
    ) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO refs (symbol_id, file_id, line, derived) VALUES (?1, ?2, ?3, 0)",
            params![symbol_id, file_id, line],
        )?;
        Ok(())
    }

    /// Record a use resolved across files (rebuilt by every `finalize`).
    pub fn insert_derived_reference(
        &self,
        symbol_id: i64,
        file_id: i64,
        line: usize,
    ) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO refs (symbol_id, file_id, line, derived) VALUES (?1, ?2, ?3, 1)",
            params![symbol_id, file_id, line],
        )?;
        Ok(())
    }

    pub fn insert_symbol_tag(&self, symbol_id: i64, tag: &str) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO symbol_tags (symbol_id, tag) VALUES (?1, ?2)",
            params![symbol_id, tag],
        )?;
        Ok(())
    }

    /// Tags such as `convex-function` set by framework adapters.
    pub fn symbol_tags(&self) -> Result<Vec<(i64, String)>, DbError> {
        let mut stmt = self.conn.prepare("SELECT symbol_id, tag FROM symbol_tags")?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Paths of every indexed file, any role.
    pub fn all_file_paths(&self) -> Result<Vec<String>, DbError> {
        let mut stmt = self.conn.prepare("SELECT path FROM files")?;
        let rows = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn insert_structural_note(
        &self,
        file_id: i64,
        kind: &str,
        name: &str,
        line: usize,
        detail: &str,
    ) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO structural_notes (file_id, kind, name, line, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![file_id, kind, name, line, detail],
        )?;
        Ok(())
    }

    /// Insert a parsed import edge.
    pub fn insert_import(
        &self,
        source_file_id: i64,
        import: &crate::analyzer::parser::ParsedImport,
    ) -> Result<i64, DbError> {
        let bindings = serde_json::to_string(&import.bindings).unwrap_or_else(|_| "[]".into());
        let members = import
            .namespace_members
            .as_ref()
            .and_then(|members| serde_json::to_string(members).ok());
        self.conn.execute(
            r#"
            INSERT INTO imports (
                source_file_id, target_path, kind, bindings, namespace, star,
                namespace_members, is_type_only, line
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            "#,
            params![
                source_file_id,
                import.target_path,
                import.kind.as_str(),
                bindings,
                import.namespace,
                import.star,
                members,
                import.is_type_only,
                import.line
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn insert_name_ref(
        &self,
        file_id: i64,
        name_ref: &crate::analyzer::parser::NameRef,
    ) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO name_refs (file_id, kind, value, specifier, line) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                file_id,
                name_ref.kind.as_str(),
                name_ref.value,
                name_ref.specifier,
                name_ref.line
            ],
        )?;
        Ok(())
    }

    pub fn name_refs(&self) -> Result<Vec<StoredNameRef>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT file_id, kind, value, specifier, line FROM name_refs")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, usize>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows
            .into_iter()
            .filter_map(|(file_id, kind, value, specifier, line)| {
                Some(StoredNameRef {
                    file_id,
                    kind: NameRefKind::parse(&kind)?,
                    value,
                    specifier,
                    line,
                })
            })
            .collect())
    }

    pub fn insert_entry(&self, file_id: i64, reason: &str) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO entries (file_id, reason) VALUES (?1, ?2)",
            params![file_id, reason],
        )?;
        Ok(())
    }

    /// Entry files and why they are entries.
    pub fn entries(&self) -> Result<Vec<(i64, String)>, DbError> {
        let mut stmt = self.conn.prepare("SELECT file_id, reason FROM entries")?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn insert_symbol_root(&self, symbol_id: i64, reason: &str) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO symbol_roots (symbol_id, reason) VALUES (?1, ?2)",
            params![symbol_id, reason],
        )?;
        Ok(())
    }

    /// Exports used by a framework or tool rather than by an import.
    pub fn symbol_roots(&self) -> Result<Vec<(i64, String)>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT symbol_id, reason FROM symbol_roots")?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Every resolved use of a symbol.
    pub fn all_refs(&self) -> Result<Vec<RefRow>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT symbol_id, file_id, line, derived FROM refs")?;
        let rows = stmt
            .query_map([], |row| {
                Ok(RefRow {
                    symbol_id: row.get(0)?,
                    file_id: row.get(1)?,
                    line: row.get(2)?,
                    derived: row.get::<_, i64>(3)? != 0,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Every file with its role.
    pub fn file_rows(&self) -> Result<Vec<FileRow>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, role, line_count FROM files")?;
        let rows = stmt
            .query_map([], |row| {
                Ok(FileRow {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    role: FileRole::parse(&row.get::<_, String>(2)?),
                    line_count: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Every symbol in source files (exported or not).
    pub fn all_symbols(&self) -> Result<Vec<Symbol>, DbError> {
        self.symbols_where("1 = 1", [])
    }

    pub fn set_import_target(&self, import_id: i64, target_file_id: i64) -> Result<(), DbError> {
        self.conn.execute(
            "UPDATE imports SET target_file_id = ?1 WHERE id = ?2",
            params![target_file_id, import_id],
        )?;
        Ok(())
    }

    /// Get all exported symbols
    pub fn get_exported_symbols(&self) -> Result<Vec<Symbol>, DbError> {
        self.symbols_where("is_exported = 1", [])
    }

    /// Get all symbols of a specific kind
    pub fn get_symbols_by_kind(&self, kind: SymbolKind) -> Result<Vec<Symbol>, DbError> {
        self.symbols_where("kind = ?1", [kind.as_str()])
    }

    fn symbols_where<P: rusqlite::Params>(
        &self,
        condition: &str,
        args: P,
    ) -> Result<Vec<Symbol>, DbError> {
        let sql = format!(
            "SELECT id, file_id, name, kind, start_line, end_line, is_exported, is_default, source_text, callee \
             FROM symbols WHERE {condition}"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let symbols = stmt
            .query_map(args, symbol_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(symbols)
    }

    /// Get all files
    pub fn get_all_files(&self) -> Result<Vec<SourceFile>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, path, hash, line_count, last_indexed FROM files WHERE role = 'source'")?;

        let files = stmt
            .query_map([], |row| {
                let last_indexed_str: String = row.get(4)?;
                let last_indexed = DateTime::parse_from_rfc3339(&last_indexed_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now());

                Ok(SourceFile {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    hash: row.get(2)?,
                    line_count: row.get(3)?,
                    last_indexed,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(files)
    }

    /// Get file path by ID
    pub fn get_file_path(&self, file_id: i64) -> Result<Option<String>, DbError> {
        let mut stmt = self.conn.prepare("SELECT path FROM files WHERE id = ?1")?;
        let result = stmt.query_row(params![file_id], |row| row.get(0));
        match result {
            Ok(path) => Ok(Some(path)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Count exports per file (for god file detection)
    pub fn count_exports_per_file(&self) -> Result<Vec<(i64, String, usize, usize)>, DbError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT f.id, f.path, f.line_count, COUNT(s.id) as export_count
            FROM files f
            LEFT JOIN symbols s ON f.id = s.file_id AND s.is_exported = 1
            WHERE f.role = 'source'
            GROUP BY f.id
            "#,
        )?;

        let results = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, usize>(2)?,
                    row.get::<_, usize>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(results)
    }

    /// Get symbols for a file
    pub fn get_file_symbols(&self, file_id: i64) -> Result<Vec<Symbol>, DbError> {
        self.symbols_where("file_id = ?1", [file_id])
    }

    /// Get import counts for barrel file detection
    pub fn get_reexport_counts(&self) -> Result<Vec<(i64, String, usize)>, DbError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT f.id, f.path,
                CASE
                    WHEN note_count > import_count THEN note_count
                    ELSE import_count
                END
            FROM (
                SELECT f.id, f.path,
                    (SELECT COUNT(*) FROM structural_notes n
                        WHERE n.file_id = f.id AND n.kind = 'reexport') AS note_count,
                    (SELECT COUNT(*) FROM imports i
                        WHERE i.source_file_id = f.id AND i.kind = 'reexport') AS import_count
                FROM files f
                WHERE f.role = 'source' AND (f.path LIKE '%/index.ts' OR f.path LIKE '%/index.js'
                   OR f.path LIKE '%/index.tsx' OR f.path LIKE '%/index.jsx'
                   OR f.path LIKE 'index.ts' OR f.path LIKE 'index.js')
            ) f
            "#,
        )?;

        let results = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, usize>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(results)
    }

    pub fn import_edges(&self) -> Result<Vec<ImportEdge>, DbError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT i.id, i.source_file_id, sf.path, i.target_file_id, tf.path,
                   i.target_path, i.kind, i.bindings, i.namespace, i.star,
                   i.namespace_members, i.is_type_only, i.line
            FROM imports i
            JOIN files sf ON sf.id = i.source_file_id
            LEFT JOIN files tf ON tf.id = i.target_file_id
            "#,
        )?;
        let edges = stmt
            .query_map([], |row| {
                let bindings: String = row.get(7)?;
                let members: Option<String> = row.get(10)?;
                Ok(ImportEdge {
                    id: row.get(0)?,
                    source_file_id: row.get(1)?,
                    source_path: row.get(2)?,
                    target_file_id: row.get(3)?,
                    target_path: row.get(4)?,
                    specifier: row.get(5)?,
                    kind: ImportKind::parse(&row.get::<_, String>(6)?),
                    bindings: serde_json::from_str(&bindings).unwrap_or_default(),
                    namespace: row.get::<_, i64>(8)? != 0,
                    star: row.get::<_, i64>(9)? != 0,
                    namespace_members: members.and_then(|text| serde_json::from_str(&text).ok()),
                    is_type_only: row.get::<_, i64>(11)? != 0,
                    line: row.get(12)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(edges)
    }

    pub fn structural_notes(&self, kind: &str) -> Result<Vec<StructuralNote>, DbError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT n.file_id, f.path, n.kind, n.name, n.line, n.detail
            FROM structural_notes n
            JOIN files f ON f.id = n.file_id
            WHERE n.kind = ?1
            "#,
        )?;
        let notes = stmt
            .query_map(params![kind], |row| {
                Ok(StructuralNote {
                    file_id: row.get(0)?,
                    file_path: row.get(1)?,
                    kind: row.get(2)?,
                    name: row.get(3)?,
                    line: row.get(4)?,
                    detail: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(notes)
    }

    pub fn find_export(&self, file_id: i64, name: &str) -> Result<Option<i64>, DbError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id FROM symbols
            WHERE file_id = ?1 AND is_exported = 1
              AND (name = ?2 OR (?2 = 'default' AND is_default = 1))
            LIMIT 1
            "#,
        )?;
        match stmt.query_row(params![file_id, name], |row| row.get(0)) {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn exported_symbol_ids(&self, file_id: i64) -> Result<Vec<i64>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM symbols WHERE file_id = ?1 AND is_exported = 1")?;
        let ids = stmt
            .query_map(params![file_id], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    pub fn find_symbol_at(
        &self,
        file_id: i64,
        name: &str,
        start_line: usize,
    ) -> Result<Option<i64>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM symbols WHERE file_id = ?1 AND name = ?2 AND start_line = ?3 LIMIT 1",
        )?;
        match stmt.query_row(params![file_id, name, start_line], |row| row.get(0)) {
            Ok(id) => Ok(Some(id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn symbols_for_embedding(&self) -> Result<Vec<(i64, String)>, DbError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, source_text FROM symbols
            WHERE kind IN ('function', 'interface', 'type_alias')
              AND length(source_text) > 0
            "#,
        )?;
        let rows = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn cached_embedding(&self, content_hash: &str) -> Result<Option<Vec<f32>>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT embedding FROM embed_cache WHERE content_hash = ?1")?;
        match stmt.query_row(params![content_hash], |row| row.get::<_, Vec<u8>>(0)) {
            Ok(blob) => Ok(Some(embeddings::from_blob(&blob))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn store_embedding(
        &self,
        symbol_id: i64,
        content_hash: &str,
        vector: &[f32],
    ) -> Result<(), DbError> {
        let blob = embeddings::to_blob(vector);
        self.conn.execute(
            "INSERT INTO embed_cache (content_hash, embedding) VALUES (?1, ?2)
             ON CONFLICT(content_hash) DO UPDATE SET embedding = excluded.embedding",
            params![content_hash, blob],
        )?;
        let _ = self.conn.execute(
            "DELETE FROM vec_symbols WHERE rowid = ?1",
            params![symbol_id],
        );
        self.conn.execute(
            "INSERT INTO vec_symbols(rowid, embedding) VALUES (?1, ?2)",
            params![symbol_id, blob],
        )?;
        Ok(())
    }

    /// Nearest symbols by cosine similarity, using the sqlite-vec index.
    pub fn nearest_ids(&self, symbol_id: i64, k: usize) -> Result<Vec<(i64, f32)>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT embedding FROM vec_symbols WHERE rowid = ?1")?;
        let blob: Vec<u8> = match stmt.query_row(params![symbol_id], |row| row.get(0)) {
            Ok(blob) => blob,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        drop(stmt);

        let mut stmt = self.conn.prepare(
            "SELECT rowid, distance FROM vec_symbols WHERE embedding MATCH ?1 AND k = ?2",
        )?;
        let rows = stmt.query_map(params![blob, k as i64], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)? as f32))
        });
        let rows = match rows {
            Ok(rows) => rows,
            Err(_) => return Ok(Vec::new()),
        };
        let mut neighbors = Vec::new();
        for row in rows {
            let (id, distance) = row?;
            if id == symbol_id {
                continue;
            }
            neighbors.push((id, embeddings::cosine_from_l2(distance)));
        }
        Ok(neighbors)
    }

    /// Check if an exported symbol has any references from another file.
    pub fn has_references(&self, symbol_id: i64) -> Result<bool, DbError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT COUNT(*) FROM refs r
            JOIN symbols s ON s.id = r.symbol_id
            WHERE r.symbol_id = ?1 AND r.file_id != s.file_id
            "#,
        )?;
        let count: i64 = stmt.query_row(params![symbol_id], |row| row.get(0))?;
        Ok(count > 0)
    }

    /// Get database statistics
    pub fn get_stats(&self) -> Result<DbStats, DbError> {
        let file_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM files WHERE role = 'source'", [], |row| row.get(0))?;
        let symbol_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM symbols", [], |row| row.get(0))?;
        let import_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM imports", [], |row| row.get(0))?;
        let total_lines: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(line_count), 0) FROM files WHERE role = 'source'",
            [],
            |row| row.get(0),
        )?;

        Ok(DbStats {
            file_count: file_count as usize,
            symbol_count: symbol_count as usize,
            import_count: import_count as usize,
            total_lines: total_lines as usize,
        })
    }
}

fn symbol_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Symbol> {
    Ok(Symbol {
        id: row.get(0)?,
        file_id: row.get(1)?,
        name: row.get(2)?,
        kind: SymbolKind::from_str(&row.get::<_, String>(3)?).unwrap_or(SymbolKind::Variable),
        start_line: row.get(4)?,
        end_line: row.get(5)?,
        is_exported: row.get(6)?,
        is_default: row.get(7)?,
        source_text: row.get(8)?,
        callee: row.get(9)?,
    })
}

#[derive(Debug, Clone)]
pub struct DbStats {
    pub file_count: usize,
    pub symbol_count: usize,
    pub import_count: usize,
    pub total_lines: usize,
}

fn ensure_sqlite_vec() {
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_database_creation() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).unwrap();
        let stats = db.get_stats().unwrap();
        assert_eq!(stats.file_count, 0);
    }

    #[test]
    fn test_file_operations() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let db = Database::open(&db_path).unwrap();

        let file_id = db.upsert_file("test.ts", "abc123", 100).unwrap();
        assert!(file_id > 0);

        let hash = db.get_file_hash("test.ts").unwrap();
        assert_eq!(hash, Some("abc123".to_string()));
    }

    #[test]
    fn sqlite_vec_finds_the_nearer_symbol() {
        let dir = tempdir().unwrap();
        let db = Database::open(&dir.path().join("vec.db")).unwrap();
        let file_id = db.upsert_file("a.ts", "h", 10).unwrap();
        let left = db
            .insert_symbol(
                file_id,
                "left",
                SymbolKind::Function,
                1,
                4,
                true,
                false,
                "function left() { return 1 }",
            )
            .unwrap();
        let right = db
            .insert_symbol(
                file_id,
                "right",
                SymbolKind::Function,
                6,
                9,
                true,
                false,
                "function right() { return 2 }",
            )
            .unwrap();
        let base = embeddings::normalize(&[1.0, 0.0, 0.0, 0.2]);
        let mut padded = vec![0.0; embeddings::EMBED_DIM];
        for (index, value) in base.iter().enumerate() {
            padded[index] = *value;
        }
        let base = embeddings::normalize(&padded);
        let mut near = base.clone();
        near[1] = 0.05;
        let near = embeddings::normalize(&near);
        let mut far = vec![0.0; embeddings::EMBED_DIM];
        far[10] = 1.0;
        db.store_embedding(left, "left", &base).unwrap();
        db.store_embedding(right, "right", &far).unwrap();
        let _ = near;
        let neighbors = db.nearest_ids(left, 2).unwrap();
        assert!(neighbors.iter().any(|(id, _)| *id == right));
    }
}
