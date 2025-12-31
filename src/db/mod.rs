//! Database storage for Keel using SQLite

use rusqlite::{Connection, params};
use std::path::{Path, PathBuf};
use thiserror::Error;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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
    pub source_text: String,
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

/// Represents an import relationship between files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Import {
    pub id: i64,
    pub source_file_id: i64,
    pub target_file_id: Option<i64>,
    pub target_path: String,
    pub imported_names: Vec<String>,
    pub is_type_only: bool,
}

/// Represents a reference to a symbol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reference {
    pub id: i64,
    pub symbol_id: i64,
    pub file_id: i64,
    pub line: usize,
}

/// Database handle for Keel
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

        let conn = Connection::open(path)?;
        let db = Self {
            conn,
            path: path.to_path_buf(),
        };
        db.init_schema()?;
        Ok(db)
    }

    /// Open database in the .keel directory of a project
    pub fn open_for_project(project_root: &Path) -> Result<Self, DbError> {
        let keel_dir = project_root.join(".keel");
        std::fs::create_dir_all(&keel_dir)?;
        let db_path = keel_dir.join("index.db");
        Self::open(&db_path)
    }

    /// Initialize the database schema
    fn init_schema(&self) -> Result<(), DbError> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY,
                path TEXT UNIQUE NOT NULL,
                hash TEXT NOT NULL,
                line_count INTEGER NOT NULL,
                last_indexed TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS symbols (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL,
                start_line INTEGER NOT NULL,
                end_line INTEGER NOT NULL,
                is_exported INTEGER NOT NULL DEFAULT 0,
                source_text TEXT NOT NULL,
                FOREIGN KEY (file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS imports (
                id INTEGER PRIMARY KEY,
                source_file_id INTEGER NOT NULL,
                target_file_id INTEGER,
                target_path TEXT NOT NULL,
                imported_names TEXT NOT NULL,
                is_type_only INTEGER NOT NULL DEFAULT 0,
                FOREIGN KEY (source_file_id) REFERENCES files(id) ON DELETE CASCADE,
                FOREIGN KEY (target_file_id) REFERENCES files(id) ON DELETE SET NULL
            );

            CREATE TABLE IF NOT EXISTS refs (
                id INTEGER PRIMARY KEY,
                symbol_id INTEGER NOT NULL,
                file_id INTEGER NOT NULL,
                line INTEGER NOT NULL,
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
            CREATE INDEX IF NOT EXISTS idx_refs_symbol ON refs(symbol_id);
            CREATE INDEX IF NOT EXISTS idx_refs_file ON refs(file_id);
            CREATE INDEX IF NOT EXISTS idx_findings_pattern ON findings(pattern_type);
            CREATE INDEX IF NOT EXISTS idx_findings_resolved ON findings(resolved);
            "#,
        )?;
        Ok(())
    }

    /// Insert or update a file record
    pub fn upsert_file(&self, path: &str, hash: &str, line_count: usize) -> Result<i64, DbError> {
        let now = Utc::now().to_rfc3339();

        self.conn.execute(
            r#"
            INSERT INTO files (path, hash, line_count, last_indexed)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(path) DO UPDATE SET
                hash = excluded.hash,
                line_count = excluded.line_count,
                last_indexed = excluded.last_indexed
            "#,
            params![path, hash, line_count, now],
        )?;

        Ok(self.conn.last_insert_rowid())
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
        let mut stmt = self.conn.prepare("SELECT hash FROM files WHERE path = ?1")?;
        let result = stmt.query_row(params![path], |row| row.get(0));
        match result {
            Ok(hash) => Ok(Some(hash)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Delete file and all related data
    pub fn delete_file(&self, path: &str) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM files WHERE path = ?1", params![path])?;
        Ok(())
    }

    /// Clear all symbols for a file (before re-indexing)
    pub fn clear_file_symbols(&self, file_id: i64) -> Result<(), DbError> {
        self.conn.execute("DELETE FROM symbols WHERE file_id = ?1", params![file_id])?;
        self.conn.execute("DELETE FROM imports WHERE source_file_id = ?1", params![file_id])?;
        Ok(())
    }

    /// Insert a symbol
    pub fn insert_symbol(
        &self,
        file_id: i64,
        name: &str,
        kind: SymbolKind,
        start_line: usize,
        end_line: usize,
        is_exported: bool,
        source_text: &str,
    ) -> Result<i64, DbError> {
        self.conn.execute(
            r#"
            INSERT INTO symbols (file_id, name, kind, start_line, end_line, is_exported, source_text)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            params![file_id, name, kind.as_str(), start_line, end_line, is_exported, source_text],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Insert an import
    pub fn insert_import(
        &self,
        source_file_id: i64,
        target_file_id: Option<i64>,
        target_path: &str,
        imported_names: &[String],
        is_type_only: bool,
    ) -> Result<i64, DbError> {
        let names_json = serde_json::to_string(imported_names).unwrap_or_default();
        self.conn.execute(
            r#"
            INSERT INTO imports (source_file_id, target_file_id, target_path, imported_names, is_type_only)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            params![source_file_id, target_file_id, target_path, names_json, is_type_only],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Get all exported symbols
    pub fn get_exported_symbols(&self) -> Result<Vec<Symbol>, DbError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, file_id, name, kind, start_line, end_line, is_exported, source_text
            FROM symbols
            WHERE is_exported = 1
            "#,
        )?;

        let symbols = stmt.query_map([], |row| {
            Ok(Symbol {
                id: row.get(0)?,
                file_id: row.get(1)?,
                name: row.get(2)?,
                kind: SymbolKind::from_str(&row.get::<_, String>(3)?).unwrap_or(SymbolKind::Variable),
                start_line: row.get(4)?,
                end_line: row.get(5)?,
                is_exported: row.get(6)?,
                source_text: row.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

        Ok(symbols)
    }

    /// Get all symbols of a specific kind
    pub fn get_symbols_by_kind(&self, kind: SymbolKind) -> Result<Vec<Symbol>, DbError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, file_id, name, kind, start_line, end_line, is_exported, source_text
            FROM symbols
            WHERE kind = ?1
            "#,
        )?;

        let symbols = stmt.query_map(params![kind.as_str()], |row| {
            Ok(Symbol {
                id: row.get(0)?,
                file_id: row.get(1)?,
                name: row.get(2)?,
                kind,
                start_line: row.get(4)?,
                end_line: row.get(5)?,
                is_exported: row.get(6)?,
                source_text: row.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

        Ok(symbols)
    }

    /// Get all files
    pub fn get_all_files(&self) -> Result<Vec<SourceFile>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, hash, line_count, last_indexed FROM files"
        )?;

        let files = stmt.query_map([], |row| {
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
            GROUP BY f.id
            "#,
        )?;

        let results = stmt.query_map([], |row| {
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
        let mut stmt = self.conn.prepare(
            r#"
            SELECT id, file_id, name, kind, start_line, end_line, is_exported, source_text
            FROM symbols
            WHERE file_id = ?1
            "#,
        )?;

        let symbols = stmt.query_map(params![file_id], |row| {
            Ok(Symbol {
                id: row.get(0)?,
                file_id: row.get(1)?,
                name: row.get(2)?,
                kind: SymbolKind::from_str(&row.get::<_, String>(3)?).unwrap_or(SymbolKind::Variable),
                start_line: row.get(4)?,
                end_line: row.get(5)?,
                is_exported: row.get(6)?,
                source_text: row.get(7)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;

        Ok(symbols)
    }

    /// Get import counts for barrel file detection
    pub fn get_reexport_counts(&self) -> Result<Vec<(i64, String, usize)>, DbError> {
        let mut stmt = self.conn.prepare(
            r#"
            SELECT f.id, f.path, COUNT(i.id) as import_count
            FROM files f
            INNER JOIN imports i ON f.id = i.source_file_id
            WHERE f.path LIKE '%/index.ts' OR f.path LIKE '%/index.js'
               OR f.path LIKE '%index.ts' OR f.path LIKE '%index.js'
            GROUP BY f.id
            "#,
        )?;

        let results = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, usize>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;

        Ok(results)
    }

    /// Check if an exported symbol has any references
    pub fn has_references(&self, symbol_id: i64) -> Result<bool, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT COUNT(*) FROM refs WHERE symbol_id = ?1"
        )?;
        let count: i64 = stmt.query_row(params![symbol_id], |row| row.get(0))?;
        Ok(count > 0)
    }

    /// Get database statistics
    pub fn get_stats(&self) -> Result<DbStats, DbError> {
        let file_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM files", [], |row| row.get(0)
        )?;
        let symbol_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM symbols", [], |row| row.get(0)
        )?;
        let import_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM imports", [], |row| row.get(0)
        )?;
        let total_lines: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(line_count), 0) FROM files", [], |row| row.get(0)
        )?;

        Ok(DbStats {
            file_count: file_count as usize,
            symbol_count: symbol_count as usize,
            import_count: import_count as usize,
            total_lines: total_lines as usize,
        })
    }
}

#[derive(Debug, Clone)]
pub struct DbStats {
    pub file_count: usize,
    pub symbol_count: usize,
    pub import_count: usize,
    pub total_lines: usize,
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
}
