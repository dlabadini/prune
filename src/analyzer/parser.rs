//! Oxc parser and semantic index for TypeScript and JavaScript.

use crate::db::SymbolKind;
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, BindingPattern, Declaration, ExportDefaultDeclarationKind, Expression,
    ImportDeclarationSpecifier, ModuleExportName, Statement, TSTypeName,
};
use oxc_ast::AstKind;
use oxc_parser::Parser as OxcParser;
use oxc_semantic::{AstNodes, Scoping, SemanticBuilder};
use oxc_span::{GetSpan, SourceType, Span};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
    /// For `const x = callee(...)`, the callee name (`query`, `internalMutation`, `z.object`).
    pub callee: Option<String>,
}

/// How a module edge was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportKind {
    /// `import ... from "x"`
    Static,
    /// `export ... from "x"`, `export * from "x"`, or `import { a } ...; export { a }`
    Reexport,
    /// `import("x")`
    Dynamic,
    /// `require("x")`
    Require,
    /// `import "x"`
    SideEffect,
    /// A string literal that names a source file (`new URL("./worker.ts", ...)`, build entry lists)
    Path,
}

impl ImportKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ImportKind::Static => "static",
            ImportKind::Reexport => "reexport",
            ImportKind::Dynamic => "dynamic",
            ImportKind::Require => "require",
            ImportKind::SideEffect => "side_effect",
            ImportKind::Path => "path",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "reexport" => ImportKind::Reexport,
            "dynamic" => ImportKind::Dynamic,
            "require" => ImportKind::Require,
            "side_effect" => ImportKind::SideEffect,
            "path" => ImportKind::Path,
            _ => ImportKind::Static,
        }
    }
}

/// One name crossing a module edge.
///
/// For imports, `imported` is the name in the target module and `local` the binding here.
/// For re-exports, `local` is the name this module exports. `imported == "*"` is a namespace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportBinding {
    pub imported: String,
    pub local: String,
    /// Declared `type`, or only ever used in type positions (erased at runtime).
    #[serde(default)]
    pub is_type: bool,
}

/// Parsed import or re-export.
#[derive(Debug, Clone)]
pub struct ParsedImport {
    pub target_path: String,
    pub kind: ImportKind,
    pub bindings: Vec<ImportBinding>,
    /// `import * as ns`, `import()`, `require()`: the whole module object is used.
    pub namespace: bool,
    /// `export * from "x"`
    pub star: bool,
    /// For a namespace import, the members accessed as `ns.member` when that is the only use.
    pub namespace_members: Option<Vec<String>>,
    /// The edge is erased at runtime (`import type`, or every binding is used only as a type).
    pub is_type_only: bool,
    pub line: usize,
}

impl ParsedImport {
    fn new(target_path: String, kind: ImportKind, line: usize) -> Self {
        Self {
            target_path,
            kind,
            bindings: Vec::new(),
            namespace: false,
            star: false,
            namespace_members: None,
            is_type_only: false,
            line,
        }
    }

    /// Imported names, for callers that do not care about aliases.
    pub fn imported_names(&self) -> Vec<String> {
        self.bindings
            .iter()
            .map(|binding| binding.imported.clone())
            .collect()
    }
}

/// A use of a binding resolved by Oxc's semantic analyzer.
#[derive(Debug, Clone)]
pub struct LocalUse {
    pub name: String,
    pub decl_line: usize,
    pub use_line: usize,
}

/// A reference that is not an import: framework member chains and string names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameRef {
    pub kind: NameRefKind,
    /// `api:comms.threads.drafts.create`, `comms/threads/internal:claim`, or `./worker.ts`
    pub value: String,
    /// The module the chain root was imported from (`../_generated/api`).
    pub specifier: Option<String>,
    pub line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameRefKind {
    /// `api.a.b.fn`, `internal.a.b.fn`, `components.c.a.fn` rooted at an imported binding.
    MemberChain,
    /// `"a/b:fn"` — Convex function names passed as strings (`makeFunctionReference`, CLI).
    FunctionString,
}

impl NameRefKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            NameRefKind::MemberChain => "member_chain",
            NameRefKind::FunctionString => "function_string",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "member_chain" => Some(NameRefKind::MemberChain),
            "function_string" => Some(NameRefKind::FunctionString),
            _ => None,
        }
    }
}

/// Everything extracted from one source file.
#[derive(Debug, Clone, Default)]
pub struct ParsedFile {
    pub symbols: Vec<ParsedSymbol>,
    pub imports: Vec<ParsedImport>,
    pub local_uses: Vec<LocalUse>,
    pub name_refs: Vec<NameRef>,
    /// Starts with `#!`: run directly, so it is an entry.
    pub executable: bool,
}

/// Imported bindings that root framework member chains.
const CHAIN_ROOTS: &[&str] = &["api", "internal", "components"];

struct ImportedBinding {
    imported: String,
    specifier: String,
    is_type: bool,
    namespace: bool,
}

/// Parse a JS/TS source file with Oxc and resolve its bindings.
pub fn parse_source(source: &str, extension: &str) -> ParsedFile {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(Path::new(&format!("file.{extension}")))
        .unwrap_or_else(|_| SourceType::default());
    let parsed = OxcParser::new(&allocator, source, source_type).parse();
    let program = parsed.program;

    let semantic = SemanticBuilder::new()
        .with_check_syntax_error(false)
        .with_build_nodes(true)
        .build(&program);
    let scoping = semantic.semantic.scoping();
    let nodes = semantic.semantic.nodes();

    let mut symbols = Vec::new();
    let mut imports = Vec::new();
    let mut name_refs = Vec::new();
    let mut export_aliases: Vec<(String, String, bool)> = Vec::new();
    let mut bound: HashMap<String, ImportedBinding> = HashMap::new();

    for stmt in &program.body {
        if let Statement::ImportDeclaration(import) = stmt {
            let parsed = import_from(source, import, scoping, nodes);
            let specifier = import.source.value.to_string();
            if let Some(specifiers) = &import.specifiers {
                for spec in specifiers {
                    let (local, imported, namespace) = match spec {
                        ImportDeclarationSpecifier::ImportSpecifier(spec) => (
                            &spec.local,
                            export_name(&spec.imported),
                            false,
                        ),
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(spec) => {
                            (&spec.local, "default".to_string(), false)
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(spec) => {
                            (&spec.local, "*".to_string(), true)
                        }
                    };
                    let is_type = parsed
                        .bindings
                        .iter()
                        .find(|binding| binding.local == local.name.as_str())
                        .is_some_and(|binding| binding.is_type)
                        || (namespace && parsed.is_type_only);
                    if !namespace && CHAIN_ROOTS.contains(&imported.as_str()) {
                        if let Some(symbol_id) = local.symbol_id.get() {
                            for reference in scoping.get_resolved_references(symbol_id) {
                                if let Some(chain) = member_chain(nodes, reference.node_id()) {
                                    name_refs.push(NameRef {
                                        kind: NameRefKind::MemberChain,
                                        value: format!("{imported}:{}", chain.join(".")),
                                        specifier: Some(specifier.clone()),
                                        line: line_at(
                                            source,
                                            nodes.kind(reference.node_id()).span().start,
                                        ),
                                    });
                                }
                            }
                        }
                    }
                    bound.insert(
                        local.name.to_string(),
                        ImportedBinding {
                            imported,
                            specifier: specifier.clone(),
                            is_type,
                            namespace,
                        },
                    );
                }
            }
            imports.push(parsed);
        }
    }

    for stmt in &program.body {
        match stmt {
            Statement::ExportAllDeclaration(export) => {
                let mut import = ParsedImport::new(
                    export.source.value.to_string(),
                    ImportKind::Reexport,
                    line_at(source, export.span.start),
                );
                import.is_type_only = export.export_kind.is_type();
                match &export.exported {
                    Some(name) => {
                        import.namespace = true;
                        import.bindings.push(ImportBinding {
                            imported: "*".to_string(),
                            local: export_name(name),
                            is_type: import.is_type_only,
                        });
                    }
                    None => import.star = true,
                }
                imports.push(import);
            }
            Statement::ExportFromDeclaration(export) => {
                let decl_type = export.export_kind.is_type();
                let mut import = ParsedImport::new(
                    export.source.value.to_string(),
                    ImportKind::Reexport,
                    line_at(source, export.span.start),
                );
                for spec in &export.specifiers {
                    let imported = export_name(&spec.local);
                    let local = export_name(&spec.exported);
                    if imported.is_empty() || local.is_empty() {
                        continue;
                    }
                    import.bindings.push(ImportBinding {
                        imported,
                        local,
                        is_type: decl_type || spec.export_kind.is_type(),
                    });
                }
                import.is_type_only =
                    decl_type || (!import.bindings.is_empty() && import.bindings.iter().all(|b| b.is_type));
                imports.push(import);
            }
            Statement::ExportDefaultDeclaration(export) => {
                if let Some(name) = default_identifier(&export.declaration) {
                    if let Some(binding) = bound.get(&name) {
                        imports.push(reexport_of_binding(
                            binding,
                            "default".to_string(),
                            false,
                            line_at(source, export.span.start),
                        ));
                        continue;
                    }
                    if symbols.iter().any(|symbol: &ParsedSymbol| symbol.name == name) {
                        export_aliases.push((name, "default".to_string(), false));
                        continue;
                    }
                }
                push_default_export(source, &export.declaration, &mut symbols);
            }
            Statement::ExportDeclaration(export) => {
                push_declaration(source, &export.declaration, true, false, &mut symbols);
            }
            Statement::ExportNamedDeclaration(export) => {
                let decl_type = export.export_kind.is_type();
                for spec in &export.specifiers {
                    let local = export_name(&spec.local);
                    let exported = export_name(&spec.exported);
                    if local.is_empty() {
                        continue;
                    }
                    if let Some(binding) = bound.get(&local) {
                        imports.push(reexport_of_binding(
                            binding,
                            exported,
                            decl_type || spec.export_kind.is_type(),
                            line_at(source, export.span.start),
                        ));
                    } else {
                        export_aliases.push((local, exported, false));
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
                push_variables_exported(source, decl, false, false, &mut symbols);
            }
            _ => {}
        }
    }

    for (local, exported, _) in &export_aliases {
        // Several declarations can share a name (overloads, `const X` + `type X`).
        for symbol in symbols.iter_mut().filter(|symbol| symbol.name == *local) {
            symbol.is_exported = true;
            if exported == "default" {
                symbol.is_default = true;
            } else if exported != local && !exported.is_empty() {
                symbol.name = exported.clone();
            }
        }
    }

    collect_expression_refs(source, nodes, &mut imports, &mut name_refs);
    for comment in &program.comments {
        let span = comment.span;
        let text = source.get(span.start as usize..span.end as usize).unwrap_or("");
        push_cli_refs(text, line_at(source, span.start), &mut name_refs);
    }

    let mut local_uses = Vec::new();
    for symbol_id in scoping.symbol_ids() {
        let name = scoping.symbol_name(symbol_id).to_string();
        let decl_line = line_at(source, scoping.symbol_span(symbol_id).start);
        for reference in scoping.get_resolved_references(symbol_id) {
            // `export { x }` and `export default x` publish the binding; they do not use it.
            if matches!(
                nodes.parent_kind(reference.node_id()),
                AstKind::ExportSpecifier(_) | AstKind::ExportDefaultDeclaration(_)
            ) {
                continue;
            }
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
        name_refs,
        executable: program.hashbang.is_some(),
    }
}

fn reexport_of_binding(
    binding: &ImportedBinding,
    exported: String,
    declared_type: bool,
    line: usize,
) -> ParsedImport {
    let mut import = ParsedImport::new(binding.specifier.clone(), ImportKind::Reexport, line);
    let is_type = declared_type || binding.is_type;
    import.namespace = binding.namespace;
    import.bindings.push(ImportBinding {
        imported: binding.imported.clone(),
        local: exported,
        is_type,
    });
    import.is_type_only = is_type;
    import
}

fn default_identifier(declaration: &ExportDefaultDeclarationKind<'_>) -> Option<String> {
    match declaration {
        ExportDefaultDeclarationKind::Identifier(ident) => Some(ident.name.to_string()),
        _ => None,
    }
}

/// Dynamic imports, `require`, and Convex function-name strings anywhere in the file.
fn collect_expression_refs(
    source: &str,
    nodes: &AstNodes<'_>,
    imports: &mut Vec<ParsedImport>,
    name_refs: &mut Vec<NameRef>,
) {
    for node in nodes.iter() {
        match node.kind() {
            AstKind::ImportExpression(expr) => {
                if let Some(target) = literal_text(&expr.source) {
                    let mut import =
                        ParsedImport::new(target, ImportKind::Dynamic, line_at(source, expr.span.start));
                    import.namespace = true;
                    imports.push(import);
                }
            }
            AstKind::CallExpression(call) => {
                if call.callee.is_specific_id("require") {
                    if let Some(Argument::StringLiteral(lit)) = call.arguments.first() {
                        let mut import = ParsedImport::new(
                            lit.value.to_string(),
                            ImportKind::Require,
                            line_at(source, call.span.start),
                        );
                        import.namespace = true;
                        imports.push(import);
                    }
                }
            }
            AstKind::StringLiteral(lit) => {
                let value = lit.value.as_str();
                if matches!(
                    nodes.parent_kind(node.id()),
                    AstKind::ImportDeclaration(_)
                        | AstKind::ExportAllDeclaration(_)
                        | AstKind::ExportFromDeclaration(_)
                        | AstKind::ImportExpression(_)
                ) {
                    continue;
                }
                push_string_ref(value, line_at(source, lit.span.start), imports, name_refs);
            }
            AstKind::TemplateLiteral(template) => {
                for quasi in &template.quasis {
                    if let Some(cooked) = &quasi.value.cooked {
                        push_cli_refs(cooked.as_str(), line_at(source, quasi.span.start), name_refs);
                    }
                }
                if template.expressions.is_empty() && template.quasis.len() == 1 {
                    if let Some(cooked) = &template.quasis[0].value.cooked {
                        push_string_ref(
                            cooked.as_str(),
                            line_at(source, template.span.start),
                            imports,
                            name_refs,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

fn push_string_ref(
    value: &str,
    line: usize,
    imports: &mut Vec<ParsedImport>,
    name_refs: &mut Vec<NameRef>,
) {
    push_cli_refs(value, line, name_refs);
    if is_function_name_string(value) {
        name_refs.push(NameRef {
            kind: NameRefKind::FunctionString,
            value: value.to_string(),
            specifier: None,
            line,
        });
    } else if is_source_path_string(value) {
        imports.push(ParsedImport::new(value.to_string(), ImportKind::Path, line));
    }
}

/// Functions named in a `convex run <module:fn>` command inside a string or comment: an
/// operator runs them by hand, which is a use.
fn push_cli_refs(text: &str, line: usize, name_refs: &mut Vec<NameRef>) {
    for name in cli_function_refs(text) {
        name_refs.push(NameRef {
            kind: NameRefKind::FunctionString,
            value: name,
            specifier: None,
            line,
        });
    }
}

pub fn cli_function_refs(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(index) = rest.find("convex run") {
        rest = &rest[index + "convex run".len()..];
        let token = rest
            .split(|ch: char| ch.is_whitespace() || matches!(ch, '`' | '\'' | '"'))
            .filter(|token| !token.is_empty())
            .find(|token| !token.starts_with('-'));
        if let Some(token) = token {
            if is_function_name_string(token) {
                found.push(token.to_string());
            }
        }
    }
    found
}

/// `comms/threads/internal:claimAuthenticated` — a module path, a colon, and an identifier.
pub fn is_function_name_string(value: &str) -> bool {
    let Some((module, function)) = value.split_once(':') else {
        return false;
    };
    !module.is_empty()
        && !function.is_empty()
        && !module.starts_with('/')
        && !module.contains("//")
        && module
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '/' | '-' | '.'))
        && module.chars().any(|ch| ch.is_ascii_alphabetic())
        && function
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        && function.chars().next().is_some_and(|ch| !ch.is_ascii_digit())
}

/// `./worker.ts` or `../lib/x.mjs` — a relative path to a JS/TS source file.
/// `./worker.ts`, `../lib/x.mjs`, or `workers/import.worker.ts` — a path to a JS/TS source.
fn is_source_path_string(value: &str) -> bool {
    if !value.contains('/')
        || value.contains(char::is_whitespace)
        || value.contains("://")
        || value.contains(['*', '$', '{', '?', '#'])
        || value.starts_with('/')
        || value.starts_with('@')
    {
        return false;
    }
    let ext = value.rsplit('.').next().unwrap_or("");
    matches!(
        ext,
        "ts" | "tsx" | "js" | "jsx" | "mts" | "mjs" | "cts" | "cjs"
    )
}

fn literal_text(expr: &Expression<'_>) -> Option<String> {
    match expr {
        Expression::StringLiteral(lit) => Some(lit.value.to_string()),
        Expression::TemplateLiteral(template)
            if template.expressions.is_empty() && template.quasis.len() == 1 =>
        {
            template.quasis[0]
                .value
                .cooked
                .as_ref()
                .map(|value| value.to_string())
        }
        _ => None,
    }
}

/// Property names hanging off a reference: `api.a.b.c` → `[a, b, c]` (outermost chain).
fn member_chain(nodes: &AstNodes<'_>, reference_node: oxc_semantic::NodeId) -> Option<Vec<String>> {
    let mut current = nodes.kind(reference_node).span();
    let mut node_id = reference_node;
    let mut chain = Vec::new();
    loop {
        let parent_id = nodes.parent_id(node_id);
        if parent_id == node_id {
            break;
        }
        match nodes.kind(parent_id) {
            AstKind::StaticMemberExpression(member) if member.object.span() == current => {
                chain.push(member.property.name.to_string());
                current = member.span;
            }
            AstKind::TSQualifiedName(name) if ts_name_span(&name.left) == current => {
                chain.push(name.right.name.to_string());
                current = name.span;
            }
            AstKind::ComputedMemberExpression(member) if member.object.span() == current => {
                match &member.expression {
                    Expression::StringLiteral(lit) => {
                        chain.push(lit.value.to_string());
                        current = member.span;
                    }
                    _ => break,
                }
            }
            AstKind::ParenthesizedExpression(_)
            | AstKind::TSNonNullExpression(_)
            | AstKind::ChainExpression(_) => {
                current = nodes.kind(parent_id).span();
            }
            _ => break,
        }
        node_id = parent_id;
    }
    if chain.is_empty() {
        None
    } else {
        Some(chain)
    }
}

fn ts_name_span(name: &TSTypeName<'_>) -> Span {
    name.span()
}

fn import_from(
    source: &str,
    import: &oxc_ast::ast::ImportDeclaration<'_>,
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
) -> ParsedImport {
    let decl_type = import.import_kind.is_type();
    let line = line_at(source, import.span.start);
    let target = import.source.value.to_string();
    let Some(specifiers) = &import.specifiers else {
        let mut parsed = ParsedImport::new(target, ImportKind::SideEffect, line);
        parsed.is_type_only = decl_type;
        return parsed;
    };
    let mut parsed = ParsedImport::new(target, ImportKind::Static, line);
    for spec in specifiers {
        match spec {
            ImportDeclarationSpecifier::ImportSpecifier(spec) => {
                let declared = decl_type || spec.import_kind.is_type();
                parsed.bindings.push(ImportBinding {
                    imported: export_name(&spec.imported),
                    local: spec.local.name.to_string(),
                    is_type: declared || only_type_uses(scoping, spec.local.symbol_id.get()),
                });
            }
            ImportDeclarationSpecifier::ImportDefaultSpecifier(spec) => {
                parsed.bindings.push(ImportBinding {
                    imported: "default".to_string(),
                    local: spec.local.name.to_string(),
                    is_type: decl_type || only_type_uses(scoping, spec.local.symbol_id.get()),
                });
            }
            ImportDeclarationSpecifier::ImportNamespaceSpecifier(spec) => {
                parsed.namespace = true;
                parsed.namespace_members =
                    spec.local.symbol_id.get().and_then(|id| namespace_members(scoping, nodes, id));
                if !decl_type && only_type_uses(scoping, spec.local.symbol_id.get()) {
                    parsed.is_type_only = true;
                }
            }
        }
    }
    let bindings_type = !parsed.bindings.is_empty() && parsed.bindings.iter().all(|b| b.is_type);
    parsed.is_type_only = decl_type
        || (parsed.namespace && parsed.is_type_only && parsed.bindings.iter().all(|b| b.is_type))
        || (!parsed.namespace && bindings_type);
    parsed
}

/// True when a binding has references and none of them read a runtime value.
fn only_type_uses(scoping: &Scoping, symbol_id: Option<oxc_semantic::SymbolId>) -> bool {
    let Some(symbol_id) = symbol_id else {
        return false;
    };
    let mut any = false;
    for reference in scoping.get_resolved_references(symbol_id) {
        any = true;
        if reference.is_value() {
            return false;
        }
    }
    any
}

/// Members read from a namespace import, when every use is `ns.member` (or `ns.Type`).
fn namespace_members(
    scoping: &Scoping,
    nodes: &AstNodes<'_>,
    symbol_id: oxc_semantic::SymbolId,
) -> Option<Vec<String>> {
    let mut members = Vec::new();
    for reference in scoping.get_resolved_references(symbol_id) {
        let node_id = reference.node_id();
        let span = nodes.kind(node_id).span();
        match nodes.parent_kind(node_id) {
            AstKind::StaticMemberExpression(member) if member.object.span() == span => {
                members.push(member.property.name.to_string());
            }
            AstKind::TSQualifiedName(name) if ts_name_span(&name.left) == span => {
                members.push(name.right.name.to_string());
            }
            _ => return None,
        }
    }
    members.sort();
    members.dedup();
    Some(members)
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
            if let Some(expr) = declaration.as_expression() {
                if let Some(symbol) = symbols.last_mut() {
                    symbol.callee = callee_name(expr);
                }
            }
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
    // Overload signatures repeat the name; keep the implementation only.
    if func.body.is_none()
        && symbols
            .iter()
            .any(|symbol| symbol.name == id.name.as_str() && symbol.kind == SymbolKind::Function)
    {
        return;
    }
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
        if is_default {
            push_named(
                source,
                "default",
                SymbolKind::Class,
                class.span,
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
        SymbolKind::Class,
        class.span,
        is_exported,
        is_default,
        symbols,
    );
}

fn push_variables_exported(
    source: &str,
    decl: &oxc_ast::ast::VariableDeclaration<'_>,
    is_exported: bool,
    is_default: bool,
    symbols: &mut Vec<ParsedSymbol>,
) {
    for declarator in &decl.declarations {
        let init = declarator.init.as_ref();
        // Arrow functions and function expressions are still callable units.
        let symbol_kind = if init.is_some_and(|expr| expr.is_function()) {
            SymbolKind::Function
        } else {
            SymbolKind::Variable
        };
        let callee = init.and_then(callee_name);
        for name in binding_names(&declarator.id) {
            push_named(
                source,
                &name,
                symbol_kind,
                declarator.span,
                is_exported,
                is_default,
                symbols,
            );
            if let Some(symbol) = symbols.last_mut() {
                symbol.callee = callee.clone();
            }
        }
    }
}

/// `query` for `query({...})`, `z.object` for `z.object({...})`, through `as`/`satisfies`.
fn callee_name(expr: &Expression<'_>) -> Option<String> {
    match expr.get_inner_expression() {
        Expression::CallExpression(call) => expression_path(&call.callee),
        _ => None,
    }
}

fn expression_path(expr: &Expression<'_>) -> Option<String> {
    match expr.get_inner_expression() {
        Expression::Identifier(ident) => Some(ident.name.to_string()),
        Expression::StaticMemberExpression(member) => {
            let object = expression_path(&member.object).or_else(|| match &member.object {
                Expression::CallExpression(call) => expression_path(&call.callee),
                _ => None,
            })?;
            Some(format!("{object}.{}", member.property.name))
        }
        Expression::CallExpression(call) => expression_path(&call.callee),
        _ => None,
    }
}

/// Names bound by a declarator, including destructuring (`export const { a, b } = x`).
fn binding_names(pattern: &BindingPattern<'_>) -> Vec<String> {
    let mut names = Vec::new();
    collect_binding_names(pattern, &mut names);
    names
}

fn collect_binding_names(pattern: &BindingPattern<'_>, names: &mut Vec<String>) {
    match pattern {
        BindingPattern::BindingIdentifier(id) => names.push(id.name.as_str().to_string()),
        BindingPattern::ObjectPattern(object) => {
            for property in &object.properties {
                collect_binding_names(&property.value, names);
            }
            if let Some(rest) = &object.rest {
                collect_binding_names(&rest.argument, names);
            }
        }
        BindingPattern::ArrayPattern(array) => {
            for element in array.elements.iter().flatten() {
                collect_binding_names(element, names);
            }
            if let Some(rest) = &array.rest {
                collect_binding_names(&rest.argument, names);
            }
        }
        BindingPattern::AssignmentPattern(assign) => collect_binding_names(&assign.left, names),
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
        callee: None,
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
        assert_eq!(parsed.imports[0].imported_names(), vec!["helper".to_string()]);
        assert!(!parsed.imports[0].is_type_only);
        assert!(parsed
            .local_uses
            .iter()
            .any(|use_site| use_site.name == "helper"));
    }

    #[test]
    fn type_only_imports_are_marked() {
        let source = r#"
import type { A } from "./a";
import { type B, C } from "./b";
import { D } from "./d";
import * as ns from "./ns";
export function f(value: D): A { return C(value as B) as A; }
export type T = ns.Shape;
"#;
        let parsed = parse_source(source, "ts");
        let by_target = |target: &str| {
            parsed
                .imports
                .iter()
                .find(|import| import.target_path == target)
                .unwrap()
        };
        assert!(by_target("./a").is_type_only);
        assert!(!by_target("./b").is_type_only);
        assert!(by_target("./b").bindings[0].is_type);
        // `D` is imported as a value but only used as a type: erased at runtime.
        assert!(by_target("./d").is_type_only);
        let ns = by_target("./ns");
        assert!(ns.namespace);
        assert!(ns.is_type_only);
        assert_eq!(ns.namespace_members, Some(vec!["Shape".to_string()]));
    }

    #[test]
    fn reexports_keep_aliases_and_stars() {
        let source = r#"
export { a as b, type C } from "./x";
export * from "./y";
export * as z from "./z";
import { local } from "./w";
export { local as published };
"#;
        let parsed = parse_source(source, "ts");
        let x = parsed
            .imports
            .iter()
            .find(|import| import.target_path == "./x")
            .unwrap();
        assert_eq!(x.kind, ImportKind::Reexport);
        assert_eq!(x.bindings[0].imported, "a");
        assert_eq!(x.bindings[0].local, "b");
        assert!(x.bindings[1].is_type);
        assert!(parsed
            .imports
            .iter()
            .any(|import| import.target_path == "./y" && import.star));
        assert!(parsed.imports.iter().any(|import| import.target_path == "./z"
            && import.bindings[0].imported == "*"
            && import.bindings[0].local == "z"));
        assert!(parsed.imports.iter().any(|import| import.target_path == "./w"
            && import.kind == ImportKind::Reexport
            && import.bindings[0].imported == "local"
            && import.bindings[0].local == "published"));
    }

    #[test]
    fn collects_convex_chains_strings_and_dynamic_imports() {
        let source = r#"
import { internal, api } from "../_generated/api";
import { makeFunctionReference } from "convex/server";
export const run = query({ handler: async (ctx) => {
    await ctx.runMutation(internal.comms.threads.drafts.create, {});
    const ref = makeFunctionReference<"query">("comms/threads/internal:claimAuthenticated");
    type Q = typeof api.inbox.list;
    const page = await import("./page");
    const worker = new URL("./worker.ts", import.meta.url);
}});
"#;
        let parsed = parse_source(source, "ts");
        let values: Vec<_> = parsed.name_refs.iter().map(|r| r.value.as_str()).collect();
        assert!(values.contains(&"internal:comms.threads.drafts.create"), "{values:?}");
        assert!(values.contains(&"api:inbox.list"), "{values:?}");
        assert!(values.contains(&"comms/threads/internal:claimAuthenticated"));
        assert!(parsed
            .imports
            .iter()
            .any(|import| import.kind == ImportKind::Dynamic && import.target_path == "./page"));
        assert!(parsed
            .imports
            .iter()
            .any(|import| import.kind == ImportKind::Path && import.target_path == "./worker.ts"));
        let run = parsed.symbols.iter().find(|s| s.name == "run").unwrap();
        assert_eq!(run.callee.as_deref(), Some("query"));
    }

    #[test]
    fn export_specifier_is_not_a_local_use() {
        let source = "function hidden() { return 1; }\nexport { hidden };\n";
        let parsed = parse_source(source, "ts");
        assert!(parsed.local_uses.is_empty(), "{:?}", parsed.local_uses);
        assert!(parsed.symbols[0].is_exported);
    }

    #[test]
    fn cli_commands_in_strings_and_comments() {
        let source = r#"
// Operational: `bunx convex run comms/senders/provisioning:createPlatformInbox`.
const fix = `bunx convex run comms/doctor:repair '{"pool":"${pool}"}' --prod`;
"#;
        let parsed = parse_source(source, "ts");
        let values: Vec<_> = parsed.name_refs.iter().map(|r| r.value.as_str()).collect();
        assert!(values.contains(&"comms/senders/provisioning:createPlatformInbox"), "{values:?}");
        assert!(values.contains(&"comms/doctor:repair"), "{values:?}");
    }

    #[test]
    fn function_name_strings() {
        assert!(is_function_name_string("comms/threads/internal:claimAuthenticated"));
        assert!(is_function_name_string("messages:send"));
        assert!(!is_function_name_string("https://example.com"));
        assert!(!is_function_name_string("a:b c"));
        assert!(!is_function_name_string("12:30"));
        assert!(!is_function_name_string("text/plain"));
    }
}
