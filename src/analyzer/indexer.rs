//! Indexer for extracting symbols and imports from source files.

use super::parser::{parse_source, ParsedFile};
use crate::analyzer::AnalyzerError;
use crate::structural::{self, StructuralHit};

/// Indexer that uses Oxc for symbols and ast-grep for structural rules.
pub struct Indexer;

impl Indexer {
    pub fn new() -> Self {
        Self
    }

    pub fn index_source(
        &mut self,
        content: &str,
        extension: &str,
    ) -> Result<IndexedFile, AnalyzerError> {
        let parsed = parse_source(content, extension);
        let structural = structural::analyze(content, extension);
        Ok(IndexedFile { parsed, structural })
    }
}

impl Default for Indexer {
    fn default() -> Self {
        Self::new()
    }
}

pub struct IndexedFile {
    pub parsed: ParsedFile,
    pub structural: Vec<StructuralHit>,
}
