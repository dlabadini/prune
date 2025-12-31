//! Tree-sitter based parser for TypeScript/JavaScript

use tree_sitter::{Language, Parser as TsParser, Node, Tree};
use crate::db::SymbolKind;

/// Parsed symbol from source code
#[derive(Debug, Clone)]
pub struct ParsedSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub start_line: usize,
    pub end_line: usize,
    pub is_exported: bool,
    pub source_text: String,
}

/// Parsed import statement
#[derive(Debug, Clone)]
pub struct ParsedImport {
    pub target_path: String,
    pub imported_names: Vec<String>,
    pub is_type_only: bool,
}

/// Tree-sitter parser wrapper
pub struct Parser {
    ts_parser: TsParser,
    typescript_lang: Language,
    javascript_lang: Language,
}

impl Parser {
    pub fn new() -> Self {
        let ts_parser = TsParser::new();
        let typescript_lang = tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into();
        let javascript_lang = tree_sitter_javascript::LANGUAGE.into();

        Self {
            ts_parser,
            typescript_lang,
            javascript_lang,
        }
    }

    /// Parse source code and extract symbols
    pub fn parse_symbols(&mut self, source: &str, extension: &str) -> Vec<ParsedSymbol> {
        let language = self.get_language(extension);
        self.ts_parser.set_language(&language).expect("Error setting language");

        let tree = match self.ts_parser.parse(source, None) {
            Some(tree) => tree,
            None => return Vec::new(),
        };

        let mut symbols = Vec::new();
        self.extract_symbols(&tree, source, &mut symbols);
        symbols
    }

    /// Parse source code and extract imports
    pub fn parse_imports(&mut self, source: &str, extension: &str) -> Vec<ParsedImport> {
        let language = self.get_language(extension);
        self.ts_parser.set_language(&language).expect("Error setting language");

        let tree = match self.ts_parser.parse(source, None) {
            Some(tree) => tree,
            None => return Vec::new(),
        };

        let mut imports = Vec::new();
        self.extract_imports(&tree, source, &mut imports);
        imports
    }

    fn get_language(&self, extension: &str) -> Language {
        match extension {
            "ts" | "tsx" | "mts" | "cts" => self.typescript_lang.clone(),
            _ => self.javascript_lang.clone(),
        }
    }

    fn extract_symbols(&self, tree: &Tree, source: &str, symbols: &mut Vec<ParsedSymbol>) {
        let root = tree.root_node();
        self.visit_node(root, source, symbols, false);
    }

    fn visit_node(&self, node: Node, source: &str, symbols: &mut Vec<ParsedSymbol>, is_exported: bool) {
        let kind = node.kind();

        // Check for export statements
        let mut exported = is_exported;
        if kind == "export_statement" {
            exported = true;
        }

        // Extract symbols based on node type
        match kind {
            "function_declaration" | "function" => {
                if let Some(name) = self.get_name_child(&node, source) {
                    symbols.push(ParsedSymbol {
                        name,
                        kind: SymbolKind::Function,
                        start_line: node.start_position().row + 1,
                        end_line: node.end_position().row + 1,
                        is_exported: exported,
                        source_text: self.get_node_text(&node, source),
                    });
                }
            }
            "arrow_function" => {
                // Arrow functions assigned to variables
                if let Some(parent) = node.parent() {
                    if parent.kind() == "variable_declarator" {
                        if let Some(name_node) = parent.child_by_field_name("name") {
                            let name = self.get_node_text(&name_node, source);
                            symbols.push(ParsedSymbol {
                                name,
                                kind: SymbolKind::Function,
                                start_line: parent.start_position().row + 1,
                                end_line: node.end_position().row + 1,
                                is_exported: exported,
                                source_text: self.get_node_text(&parent.parent().unwrap_or(parent), source),
                            });
                        }
                    }
                }
            }
            "class_declaration" | "class" => {
                if let Some(name) = self.get_name_child(&node, source) {
                    symbols.push(ParsedSymbol {
                        name,
                        kind: SymbolKind::Class,
                        start_line: node.start_position().row + 1,
                        end_line: node.end_position().row + 1,
                        is_exported: exported,
                        source_text: self.get_node_text(&node, source),
                    });
                }
            }
            "interface_declaration" => {
                if let Some(name) = self.get_name_child(&node, source) {
                    symbols.push(ParsedSymbol {
                        name,
                        kind: SymbolKind::Interface,
                        start_line: node.start_position().row + 1,
                        end_line: node.end_position().row + 1,
                        is_exported: exported,
                        source_text: self.get_node_text(&node, source),
                    });
                }
            }
            "type_alias_declaration" => {
                if let Some(name) = self.get_name_child(&node, source) {
                    symbols.push(ParsedSymbol {
                        name,
                        kind: SymbolKind::TypeAlias,
                        start_line: node.start_position().row + 1,
                        end_line: node.end_position().row + 1,
                        is_exported: exported,
                        source_text: self.get_node_text(&node, source),
                    });
                }
            }
            "enum_declaration" => {
                if let Some(name) = self.get_name_child(&node, source) {
                    symbols.push(ParsedSymbol {
                        name,
                        kind: SymbolKind::Enum,
                        start_line: node.start_position().row + 1,
                        end_line: node.end_position().row + 1,
                        is_exported: exported,
                        source_text: self.get_node_text(&node, source),
                    });
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                // Only track exported variables at module level
                if exported {
                    for i in 0..node.named_child_count() {
                        if let Some(child) = node.named_child(i) {
                            if child.kind() == "variable_declarator" {
                                if let Some(name_node) = child.child_by_field_name("name") {
                                    let name = self.get_node_text(&name_node, source);
                                    // Skip if this is an arrow function (handled above)
                                    let is_function = child.named_child_count() > 1
                                        && child.named_child(1).map(|n| n.kind() == "arrow_function").unwrap_or(false);

                                    if !is_function {
                                        symbols.push(ParsedSymbol {
                                            name,
                                            kind: SymbolKind::Variable,
                                            start_line: node.start_position().row + 1,
                                            end_line: node.end_position().row + 1,
                                            is_exported: true,
                                            source_text: self.get_node_text(&node, source),
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "module" | "namespace_declaration" => {
                if let Some(name) = self.get_name_child(&node, source) {
                    symbols.push(ParsedSymbol {
                        name,
                        kind: SymbolKind::Namespace,
                        start_line: node.start_position().row + 1,
                        end_line: node.end_position().row + 1,
                        is_exported: exported,
                        source_text: self.get_node_text(&node, source),
                    });
                }
            }
            _ => {}
        }

        // Recurse into children
        for i in 0..node.named_child_count() {
            if let Some(child) = node.named_child(i) {
                self.visit_node(child, source, symbols, exported);
            }
        }
    }

    fn extract_imports(&self, tree: &Tree, source: &str, imports: &mut Vec<ParsedImport>) {
        let root = tree.root_node();
        self.visit_node_for_imports(root, source, imports);
    }

    fn visit_node_for_imports(&self, node: Node, source: &str, imports: &mut Vec<ParsedImport>) {
        let kind = node.kind();

        if kind == "import_statement" {
            if let Some(import) = self.parse_import_statement(&node, source) {
                imports.push(import);
            }
        }

        // Also handle export ... from statements (re-exports)
        if kind == "export_statement" {
            if let Some(source_node) = node.child_by_field_name("source") {
                let target_path = self.get_string_value(&source_node, source);
                let mut imported_names = Vec::new();

                // Get exported names
                for i in 0..node.named_child_count() {
                    if let Some(child) = node.named_child(i) {
                        if child.kind() == "export_clause" {
                            for j in 0..child.named_child_count() {
                                if let Some(spec) = child.named_child(j) {
                                    if spec.kind() == "export_specifier" {
                                        if let Some(name_node) = spec.child_by_field_name("name") {
                                            imported_names.push(self.get_node_text(&name_node, source));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if !target_path.is_empty() {
                    imports.push(ParsedImport {
                        target_path,
                        imported_names,
                        is_type_only: false,
                    });
                }
            }
        }

        // Recurse
        for i in 0..node.named_child_count() {
            if let Some(child) = node.named_child(i) {
                self.visit_node_for_imports(child, source, imports);
            }
        }
    }

    fn parse_import_statement(&self, node: &Node, source: &str) -> Option<ParsedImport> {
        let mut target_path = String::new();
        let mut imported_names = Vec::new();
        let mut is_type_only = false;

        // Check if this is a type-only import
        let node_text = self.get_node_text(node, source);
        if node_text.contains("import type") {
            is_type_only = true;
        }

        // Get the source path
        if let Some(source_node) = node.child_by_field_name("source") {
            target_path = self.get_string_value(&source_node, source);
        }

        // Get imported names
        for i in 0..node.named_child_count() {
            if let Some(child) = node.named_child(i) {
                match child.kind() {
                    "import_clause" => {
                        // Handle default import and named imports
                        for j in 0..child.named_child_count() {
                            if let Some(import_child) = child.named_child(j) {
                                match import_child.kind() {
                                    "identifier" => {
                                        imported_names.push(self.get_node_text(&import_child, source));
                                    }
                                    "named_imports" => {
                                        for k in 0..import_child.named_child_count() {
                                            if let Some(spec) = import_child.named_child(k) {
                                                if spec.kind() == "import_specifier" {
                                                    if let Some(name_node) = spec.child_by_field_name("name") {
                                                        imported_names.push(self.get_node_text(&name_node, source));
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    "namespace_import" => {
                                        // import * as name
                                        if let Some(name_node) = import_child.child_by_field_name("name") {
                                            imported_names.push(format!("* as {}", self.get_node_text(&name_node, source)));
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if target_path.is_empty() {
            return None;
        }

        Some(ParsedImport {
            target_path,
            imported_names,
            is_type_only,
        })
    }

    fn get_name_child(&self, node: &Node, source: &str) -> Option<String> {
        node.child_by_field_name("name")
            .map(|n| self.get_node_text(&n, source))
    }

    fn get_node_text(&self, node: &Node, source: &str) -> String {
        let start = node.start_byte();
        let end = node.end_byte();
        source.get(start..end).unwrap_or("").to_string()
    }

    fn get_string_value(&self, node: &Node, source: &str) -> String {
        let text = self.get_node_text(node, source);
        // Remove quotes
        text.trim_matches(|c| c == '"' || c == '\'' || c == '`').to_string()
    }
}

impl Default for Parser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_function() {
        let mut parser = Parser::new();
        let source = r#"
            export function greet(name: string): string {
                return `Hello, ${name}!`;
            }
        "#;

        let symbols = parser.parse_symbols(source, "ts");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "greet");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert!(symbols[0].is_exported);
    }

    #[test]
    fn test_parse_interface() {
        let mut parser = Parser::new();
        let source = r#"
            export interface User {
                id: string;
                name: string;
                email: string;
            }
        "#;

        let symbols = parser.parse_symbols(source, "ts");
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "User");
        assert_eq!(symbols[0].kind, SymbolKind::Interface);
    }

    #[test]
    fn test_parse_imports() {
        let mut parser = Parser::new();
        let source = r#"
            import { useState, useEffect } from 'react';
            import type { User } from './types';
            import * as utils from './utils';
        "#;

        let imports = parser.parse_imports(source, "ts");
        assert_eq!(imports.len(), 3);
        assert_eq!(imports[0].target_path, "react");
        assert_eq!(imports[0].imported_names, vec!["useState", "useEffect"]);
        assert!(imports[1].is_type_only);
    }
}
