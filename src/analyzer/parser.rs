//! Oxc parser and semantic index for TypeScript and JavaScript.

use crate::db::SymbolKind;
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    BindingPattern, Declaration, ExportDefaultDeclarationKind, ImportDeclarationSpecifier,
    ModuleExportName, Statement, VariableDeclarationKind,
};
use oxc_parser::Parser as OxcParser;
use oxc_semantic::SemanticBuilder;
use oxc_span::{GetSpan, SourceType};
use std::path::Path;

/// Parsed symbol from source code.
#[derive(Debug, Clone)]
pub struct ParsedSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub start_line: usize,
    pub end_line: usize,
    pub is_exported: bool,
    pub is_default: bool,
    pub source_text: String,
}

/// Parsed import or re-export.
#[derive(Debug, Clone)]
pub struct ParsedImport {
    pub target_path: String,
    pub imported_names: Vec<String>,
    pub is_type_only: bool,
    pub is_reexport: bool,
    pub import_all: bool,
    pub line: usize,
}

/// A use of a binding resolved by Oxc's semantic analyzer.
#[derive(Debug, Clone)]
pub struct LocalUse {
    pub name: String,
    pub decl_line: usize,
    pub use_line: usize,
}

/// Everything extracted from one source file.
#[derive(Debug, Clone, Default)]
pub struct ParsedFile {
    pub symbols: Vec<ParsedSymbol>,
    pub imports: Vec<ParsedImport>,
    pub local_uses: Vec<LocalUse>,
}

/// Parse a JS/TS source file with Oxc and resolve its bindings.
pub fn parse_source(source: &str, extension: &str) -> ParsedFile {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(Path::new(&format!("file.{extension}")))
        .unwrap_or_else(|_| SourceType::default());
    let parsed = OxcParser::new(&allocator, source, source_type).parse();
    let program = parsed.program;

    let mut symbols = Vec::new();
    let mut imports = Vec::new();
    let mut export_aliases: Vec<(String, String)> = Vec::new();

    for stmt in &program.body {
        match stmt {
            Statement::ImportDeclaration(import) => {
                imports.push(import_from(source, import, false));
            }
            Statement::ExportAllDeclaration(export) => {
                imports.push(ParsedImport {
                    target_path: export.source.value.to_string(),
                    imported_names: Vec::new(),
                    is_type_only: false,
                    is_reexport: true,
                    import_all: true,
                    line: line_at(source, export.span.start),
                });
            }
            Statement::ExportFromDeclaration(export) => {
                let names = export
                    .specifiers
                    .iter()
                    .map(|spec| export_name(&spec.exported))
                    .filter(|name| !name.is_empty())
                    .collect::<Vec<_>>();
                imports.push(ParsedImport {
                    target_path: export.source.value.to_string(),
                    imported_names: names,
                    is_type_only: false,
                    is_reexport: true,
                    import_all: false,
                    line: line_at(source, export.span.start),
                });
            }
            Statement::ExportDefaultDeclaration(export) => {
                push_default_export(source, &export.declaration, &mut symbols);
            }
            Statement::ExportDeclaration(export) => {
                push_declaration(source, &export.declaration, true, false, &mut symbols);
            }
            Statement::ExportNamedDeclaration(export) => {
                for spec in &export.specifiers {
                    let local = export_name(&spec.local);
                    let exported = export_name(&spec.exported);
                    if !local.is_empty() {
                        export_aliases.push((local, exported));
                    }
                }
            }
            Statement::FunctionDeclaration(func) => {
                push_function(source, func, false, false, &mut symbols);
            }
            Statement::ClassDeclaration(class) => {
                push_class(source, class, false, false, &mut symbols);
            }
            Statement::TSInterfaceDeclaration(interface) => {
                push_named(
                    source,
                    interface.id.name.as_str(),
                    SymbolKind::Interface,
                    interface.span,
                    false,
                    false,
                    &mut symbols,
                );
            }
            Statement::TSTypeAliasDeclaration(alias) => {
                push_named(
                    source,
                    alias.id.name.as_str(),
                    SymbolKind::TypeAlias,
                    alias.span,
                    false,
                    false,
                    &mut symbols,
                );
            }
            Statement::TSEnumDeclaration(enum_decl) => {
                push_named(
                    source,
                    enum_decl.id.name.as_str(),
                    SymbolKind::Enum,
                    enum_decl.span,
                    false,
                    false,
                    &mut symbols,
                );
            }
            Statement::VariableDeclaration(decl) => {
                push_variables(source, decl, false, &mut symbols);
            }
            _ => {}
        }
    }

    for (local, exported) in &export_aliases {
        if let Some(symbol) = symbols.iter_mut().find(|symbol| symbol.name == *local) {
            symbol.is_exported = true;
            if exported != local && !exported.is_empty() {
                symbol.name = exported.clone();
            }
        }
    }

    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(false)
        .with_build_nodes(true)
        .build(&program);
    let scoping = semantic.semantic.scoping();
    let nodes = semantic.semantic.nodes();
    let mut local_uses = Vec::new();
    for symbol_id in scoping.symbol_ids() {
        let name = scoping.symbol_name(symbol_id).to_string();
        let decl_line = line_at(source, scoping.symbol_span(symbol_id).start);
        for reference in scoping.get_resolved_references(symbol_id) {
            let use_line = line_at(source, nodes.kind(reference.node_id()).span().start);
            if use_line != decl_line {
                local_uses.push(LocalUse {
                    name: name.clone(),
                    decl_line,
                    use_line,
                });
            }
        }
    }

    ParsedFile {
        symbols,
        imports,
        local_uses,
    }
}

fn import_from(
    source: &str,
    import: &oxc_ast::ast::ImportDeclaration<'_>,
    is_reexport: bool,
) -> ParsedImport {
    let mut names = Vec::new();
    let mut import_all = false;
    if let Some(specifiers) = &import.specifiers {
        for spec in specifiers {
            match spec {
                ImportDeclarationSpecifier::ImportSpecifier(spec) => {
                    names.push(export_name(&spec.imported));
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => {
                    names.push("default".to_string());
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {
                    import_all = true;
                }
            }
        }
    }
    ParsedImport {
        target_path: import.source.value.to_string(),
        imported_names: names,
        is_type_only: false,
        is_reexport,
        import_all,
        line: line_at(source, import.span.start),
    }
}

fn push_default_export(
    source: &str,
    declaration: &ExportDefaultDeclarationKind<'_>,
    symbols: &mut Vec<ParsedSymbol>,
) {
    match declaration {
        ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
            push_function(source, func, true, true, symbols);
        }
        ExportDefaultDeclarationKind::ClassDeclaration(class) => {
            push_class(source, class, true, true, symbols);
        }
        ExportDefaultDeclarationKind::TSInterfaceDeclaration(interface) => {
            push_named(
                source,
                interface.id.name.as_str(),
                SymbolKind::Interface,
                interface.span,
                true,
                true,
                symbols,
            );
        }
        _ => {
            let span = declaration.span();
            push_named(
                source,
                "default",
                SymbolKind::Variable,
                span,
                true,
                true,
                symbols,
            );
        }
    }
}

fn push_declaration(
    source: &str,
    declaration: &Declaration<'_>,
    is_exported: bool,
    is_default: bool,
    symbols: &mut Vec<ParsedSymbol>,
) {
    match declaration {
        Declaration::FunctionDeclaration(func) => {
            push_function(source, func, is_exported, is_default, symbols);
        }
        Declaration::ClassDeclaration(class) => {
            push_class(source, class, is_exported, is_default, symbols);
        }
        Declaration::VariableDeclaration(decl) => {
            push_variables_exported(source, decl, is_exported, is_default, symbols);
        }
        Declaration::TSInterfaceDeclaration(interface) => {
            push_named(
                source,
                interface.id.name.as_str(),
                SymbolKind::Interface,
                interface.span,
                is_exported,
                is_default,
                symbols,
            );
        }
        Declaration::TSTypeAliasDeclaration(alias) => {
            push_named(
                source,
                alias.id.name.as_str(),
                SymbolKind::TypeAlias,
                alias.span,
                is_exported,
                is_default,
                symbols,
            );
        }
        Declaration::TSEnumDeclaration(enum_decl) => {
            push_named(
                source,
                enum_decl.id.name.as_str(),
                SymbolKind::Enum,
                enum_decl.span,
                is_exported,
                is_default,
                symbols,
            );
        }
        _ => {}
    }
}

fn push_function(
    source: &str,
    func: &oxc_ast::ast::Function<'_>,
    is_exported: bool,
    is_default: bool,
    symbols: &mut Vec<ParsedSymbol>,
) {
    let Some(id) = &func.id else {
        if is_default {
            push_named(
                source,
                "default",
                SymbolKind::Function,
                func.span,
                true,
                true,
                symbols,
            );
        }
        return;
    };
    push_named(
        source,
        id.name.as_str(),
        SymbolKind::Function,
        func.span,
        is_exported,
        is_default,
        symbols,
    );
}

fn push_class(
    source: &str,
    class: &oxc_ast::ast::Class<'_>,
    is_exported: bool,
    is_default: bool,
    symbols: &mut Vec<ParsedSymbol>,
) {
    let Some(id) = &class.id else {
        return;
    };
    push_named(
        source,
        id.name.as_str(),
        SymbolKind::Class,
        class.span,
        is_exported,
        is_default,
        symbols,
    );
}

fn push_variables(
    source: &str,
    decl: &oxc_ast::ast::VariableDeclaration<'_>,
    is_exported: bool,
    symbols: &mut Vec<ParsedSymbol>,
) {
    push_variables_exported(source, decl, is_exported, false, symbols);
}

fn push_variables_exported(
    source: &str,
    decl: &oxc_ast::ast::VariableDeclaration<'_>,
    is_exported: bool,
    is_default: bool,
    symbols: &mut Vec<ParsedSymbol>,
) {
    let kind = match decl.kind {
        VariableDeclarationKind::Var
        | VariableDeclarationKind::Let
        | VariableDeclarationKind::Const => SymbolKind::Variable,
        _ => SymbolKind::Variable,
    };
    for declarator in &decl.declarations {
        if let Some(name) = binding_name(&declarator.id) {
            // Arrow functions and function expressions are still callable units.
            let symbol_kind = if declarator
                .init
                .as_ref()
                .is_some_and(|expr| expr.is_function())
            {
                SymbolKind::Function
            } else {
                kind
            };
            push_named(
                source,
                &name,
                symbol_kind,
                declarator.span,
                is_exported,
                is_default,
                symbols,
            );
        }
    }
}

fn binding_name(pattern: &BindingPattern<'_>) -> Option<String> {
    match pattern {
        BindingPattern::BindingIdentifier(id) => Some(id.name.as_str().to_string()),
        _ => None,
    }
}

fn push_named(
    source: &str,
    name: &str,
    kind: SymbolKind,
    span: oxc_span::Span,
    is_exported: bool,
    is_default: bool,
    symbols: &mut Vec<ParsedSymbol>,
) {
    if name.is_empty() {
        return;
    }
    let start = span.start as usize;
    let end = (span.end as usize).min(source.len());
    let text = source.get(start..end).unwrap_or("").to_string();
    symbols.push(ParsedSymbol {
        name: name.to_string(),
        kind,
        start_line: line_at(source, span.start),
        end_line: line_at(source, span.end.saturating_sub(1)),
        is_exported,
        is_default,
        source_text: text,
    });
}

fn export_name(name: &ModuleExportName<'_>) -> String {
    match name {
        ModuleExportName::IdentifierName(id) => id.name.as_str().to_string(),
        ModuleExportName::IdentifierReference(id) => id.name.as_str().to_string(),
        ModuleExportName::StringLiteral(lit) => lit.value.to_string(),
    }
}

fn line_at(source: &str, offset: u32) -> usize {
    let end = (offset as usize).min(source.len());
    source[..end].bytes().filter(|byte| *byte == b'\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exports_imports_and_references() {
        let source = r#"
import { helper } from "./helper";

export function used(value: string) {
    return helper(value);
}

export function unused() {
    return 1;
}
"#;
        let parsed = parse_source(source, "ts");
        let used = parsed
            .symbols
            .iter()
            .find(|symbol| symbol.name == "used")
            .unwrap();
        assert!(used.is_exported);
        assert_eq!(used.kind, SymbolKind::Function);
        assert!(parsed.symbols.iter().any(|symbol| symbol.name == "unused"));
        assert_eq!(parsed.imports.len(), 1);
        assert_eq!(parsed.imports[0].imported_names, vec!["helper".to_string()]);
        assert!(parsed
            .local_uses
            .iter()
            .any(|use_site| use_site.name == "helper"));
    }
}
