//! Indexer for extracting symbols and imports from source files

use super::parser::{ParsedImport, ParsedSymbol, Parser};
use crate::analyzer::AnalyzerError;

/// Indexer that uses tree-sitter parser to extract code structure
pub struct Indexer {
    parser: Parser,
}

impl Indexer {
    pub fn new() -> Self {
        Self {
            parser: Parser::new(),
        }
    }

    /// Parse a file and extract symbols
    pub fn parse_file(&mut self, content: &str, extension: &str) -> Result<Vec<ParsedSymbol>, AnalyzerError> {
        Ok(self.parser.parse_symbols(content, extension))
    }

    /// Parse a file and extract imports
    pub fn parse_imports(&mut self, content: &str, extension: &str) -> Result<Vec<ParsedImport>, AnalyzerError> {
        Ok(self.parser.parse_imports(content, extension))
    }
}

impl Default for Indexer {
    fn default() -> Self {
        Self::new()
    }
}
