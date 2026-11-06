//! Safe autofix: drop unnecessary `export`, delete unreachable files, rename across the graph.
//!
//! Dry run by default. When applying, each fix (one file's un-exports, one deleted file, one
//! rename) is written, the configured check command runs, and the fix is reverted if it fails.

use crate::analyzer::parser::ImportKind;
use crate::db::Database;
use crate::graph::ModuleGraph;
use crate::patterns::{Finding, PatternType};
use crate::verify::run_check;
use oxc_allocator::Allocator;
use oxc_ast::ast::{ImportDeclarationSpecifier, ModuleExportName, Statement};
use oxc_ast::AstKind;
use oxc_parser::Parser as OxcParser;
use oxc_semantic::SemanticBuilder;
use oxc_span::{GetSpan, SourceType, Span};
use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FixKind {
    /// Remove `export` from a declaration only its own file uses.
    Unexport,
    /// Delete a file no entry reaches.
    DeleteFile,
}

impl FixKind {
    pub fn parse_list(values: &[String]) -> Vec<FixKind> {
        let mut kinds: Vec<FixKind> = values
            .iter()
            .filter_map(|value| match value.as_str() {
                "unexport" => Some(FixKind::Unexport),
                "delete-file" | "delete" => Some(FixKind::DeleteFile),
                _ => None,
            })
            .collect();
        if kinds.is_empty() {
            kinds = vec![FixKind::Unexport, FixKind::DeleteFile];
        }
        kinds
    }
}

pub struct FixOptions {
    pub apply: bool,
    pub check: Option<String>,
    pub kinds: Vec<FixKind>,
    pub path_prefix: Option<String>,
}

/// One unit that is applied and verified together.
#[derive(Debug, Clone)]
pub struct PlannedFix {
    pub kind: FixKind,
    pub file: String,
    /// Symbols un-exported (empty for a deleted file).
    pub symbols: Vec<String>,
    /// Why a candidate was not planned.
    pub skipped: Vec<(String, String)>,
    pub new_contents: Option<String>,
    /// For a deletion: every file deleted together (unreachable files that import each other
    /// go as one unit, so the check never sees half a group).
    pub group: Vec<String>,
}

#[derive(Debug, Default)]
pub struct FixReport {
    pub planned: Vec<PlannedFix>,
    pub applied: Vec<String>,
    pub reverted: Vec<(String, String)>,
}

impl FixReport {
    pub fn render(&self, applied: bool) -> String {
        let mut out = String::new();
        for fix in &self.planned {
            match fix.kind {
                FixKind::Unexport if !fix.symbols.is_empty() => {
                    out.push_str(&format!(
                        "unexport  {}: {}\n",
                        fix.file,
                        fix.symbols.join(", ")
                    ));
                }
                FixKind::DeleteFile => {
                    out.push_str(&format!("delete    {}\n", fix.group.join(" + ")));
                }
                _ => {}
            }
            for (name, reason) in &fix.skipped {
                out.push_str(&format!("  skip {name}: {reason}\n"));
            }
        }
        if applied {
            out.push_str(&format!(
                "\nApplied {} fix(es); reverted {}.\n",
                self.applied.len(),
                self.reverted.len()
            ));
            for (file, why) in &self.reverted {
                out.push_str(&format!("  reverted {file}: {why}\n"));
            }
        } else {
            out.push_str("\nDry run. Re-run with --apply (and --check \"<cmd>\") to write.\n");
        }
        out
    }
}

/// Plan (and with `apply`, perform) fixes for dead-export findings.
pub fn run(
    root: &Path,
    db: &Database,
    findings: &[Finding],
    options: &FixOptions,
) -> io::Result<FixReport> {
    let graph = ModuleGraph::load(db).map_err(|error| io::Error::other(error.to_string()))?;
    let mut unexports: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();
    let mut deletes: Vec<String> = Vec::new();
    for finding in findings {
        if finding.pattern_type != PatternType::DeadExports {
            continue;
        }
        for symbol in &finding.symbols {
            if let Some(prefix) = &options.path_prefix {
                if !symbol.file.starts_with(prefix.as_str()) {
                    continue;
                }
            }
            match symbol.usage.as_str() {
                "same_file_only" if options.kinds.contains(&FixKind::Unexport) => {
                    unexports
                        .entry(symbol.file.clone())
                        .or_default()
                        .push((symbol.name.clone(), symbol.line));
                }
                "unreachable" if symbol.name == "*" && options.kinds.contains(&FixKind::DeleteFile) => {
                    deletes.push(symbol.file.clone());
                }
                _ => {}
            }
        }
    }

    let mut report = FixReport::default();
    for (file, symbols) in unexports {
        let reexported = reexported_names(&graph, &file);
        let Ok(source) = std::fs::read_to_string(root.join(&file)) else {
            continue;
        };
        let (new_contents, done, skipped) = unexport_source(&source, &symbols, &reexported);
        report.planned.push(PlannedFix {
            kind: FixKind::Unexport,
            file,
            symbols: done,
            skipped,
            new_contents: Some(new_contents),
            group: Vec::new(),
        });
    }
    deletes.sort();
    deletes.dedup();
    for group in delete_groups(&graph, &deletes) {
        report.planned.push(PlannedFix {
            kind: FixKind::DeleteFile,
            file: group[0].clone(),
            symbols: Vec::new(),
            skipped: Vec::new(),
            new_contents: None,
            group,
        });
    }

    if options.apply {
        let Some(command) = options.check.as_deref() else {
            return Err(io::Error::other(
                "--apply needs a check command (--check \"<cmd>\" or [fix] check in .prunerc); \
                 pass --check true to apply without verification",
            ));
        };
        for fix in &report.planned {
            if fix.kind == FixKind::Unexport && fix.symbols.is_empty() {
                continue;
            }
            let files: Vec<String> = if fix.group.is_empty() {
                vec![fix.file.clone()]
            } else {
                fix.group.clone()
            };
            let mut originals = Vec::new();
            for file in &files {
                let path = root.join(file);
                originals.push((path.clone(), std::fs::read(&path)?));
                match &fix.new_contents {
                    Some(contents) => std::fs::write(&path, contents)?,
                    None => std::fs::remove_file(&path)?,
                }
            }
            let outcome = run_check(root, command)?;
            if outcome.passed {
                report.applied.extend(files);
            } else {
                for (path, original) in &originals {
                    std::fs::write(path, original)?;
                }
                report
                    .reverted
                    .push((files.join(", "), format!("`{command}` failed")));
            }
        }
    }
    Ok(report)
}

/// Unreachable files that import each other, grouped (connected through imports).
fn delete_groups(graph: &ModuleGraph, files: &[String]) -> Vec<Vec<String>> {
    let index: std::collections::HashMap<&str, usize> =
        files.iter().enumerate().map(|(i, f)| (f.as_str(), i)).collect();
    let mut parent: Vec<usize> = (0..files.len()).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut root = i;
        while parent[root] != root {
            root = parent[root];
        }
        parent[i] = root;
        root
    }
    for edge in &graph.edges {
        let Some(target) = edge.target_file_id else {
            continue;
        };
        let (Some(&a), Some(&b)) = (
            index.get(edge.source_path.as_str()),
            index.get(graph.path(target)),
        ) else {
            continue;
        };
        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
        if ra != rb {
            parent[ra] = rb;
        }
    }
    let mut groups: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (i, file) in files.iter().enumerate() {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(file.clone());
    }
    let mut out: Vec<Vec<String>> = groups.into_values().collect();
    out.sort();
    out
}

/// Names of `file` that a barrel re-exports by name (un-exporting them would break it).
fn reexported_names(graph: &ModuleGraph, file: &str) -> HashSet<String> {
    let Some(file_id) = graph.file_id(file) else {
        return HashSet::new();
    };
    graph
        .edges
        .iter()
        .filter(|edge| edge.kind == ImportKind::Reexport && edge.target_file_id == Some(file_id))
        .flat_map(|edge| edge.bindings.iter().map(|binding| binding.imported.clone()))
        .collect()
}

const DECLARATION_KEYWORDS: &[&str] = &[
    "async function",
    "function",
    "const",
    "let",
    "var",
    "class",
    "abstract class",
    "interface",
    "type",
    "enum",
    "const enum",
    "declare",
];

/// Remove `export ` from the declarations of `symbols`. Returns (contents, done, skipped).
fn unexport_source(
    source: &str,
    symbols: &[(String, usize)],
    reexported: &HashSet<String>,
) -> (String, Vec<String>, Vec<(String, String)>) {
    let mut lines: Vec<String> = source.split('\n').map(String::from).collect();
    let mut done = Vec::new();
    let mut skipped = Vec::new();
    for (name, line) in symbols {
        if reexported.contains(name) {
            skipped.push((name.clone(), "a barrel re-exports it by name".into()));
            continue;
        }
        let Some(text) = line.checked_sub(1).and_then(|index| lines.get_mut(index)) else {
            skipped.push((name.clone(), "line not found".into()));
            continue;
        };
        let indent_len = text.len() - text.trim_start().len();
        let rest = &text[indent_len..];
        let Some(after) = rest.strip_prefix("export ") else {
            skipped.push((name.clone(), "exported through an export list".into()));
            continue;
        };
        let declares = DECLARATION_KEYWORDS
            .iter()
            .any(|keyword| after.starts_with(&format!("{keyword} ")));
        let names_symbol = after
            .split(|ch: char| !(ch.is_alphanumeric() || ch == '_' || ch == '$'))
            .any(|word| word == name);
        if !declares || !names_symbol || after.starts_with("default") {
            skipped.push((name.clone(), "declaration is not a plain `export <decl>` line".into()));
            continue;
        }
        *text = format!("{}{}", &text[..indent_len], after);
        done.push(name.clone());
    }
    (lines.join("\n"), done, skipped)
}

/// A planned rename: replacements per file.
#[derive(Debug, Clone)]
pub struct RenamePlan {
    pub from: String,
    pub to: String,
    pub edits: BTreeMap<String, Vec<(Span, String)>>,
    pub refused: Option<String>,
}

impl RenamePlan {
    pub fn render(&self) -> String {
        if let Some(reason) = &self.refused {
            return format!("rename {} → {}: refused: {reason}\n", self.from, self.to);
        }
        let mut out = format!(
            "rename {} → {} touches {} file(s):\n",
            self.from,
            self.to,
            self.edits.len()
        );
        for (file, edits) in &self.edits {
            out.push_str(&format!("  {file} ({} edit(s))\n", edits.len()));
        }
        out
    }

    pub fn files(&self) -> Vec<String> {
        self.edits.keys().cloned().collect()
    }
}

/// Plan renaming `from` (declared in `file`) to `to` in its file, barrels, and importers.
pub fn plan_rename(db: &Database, root: &Path, file: &str, from: &str, to: &str) -> io::Result<RenamePlan> {
    let report = crate::graph::usage::UsageReport::load(db)
        .map_err(|error| io::Error::other(error.to_string()))?;
    let graph = &report.graph;
    let mut plan = RenamePlan {
        from: from.to_string(),
        to: to.to_string(),
        edits: BTreeMap::new(),
        refused: None,
    };
    let Some(file_id) = graph.file_id(file) else {
        plan.refused = Some(format!("{file} is not indexed"));
        return Ok(plan);
    };
    let Some(symbol_id) = graph
        .exported(file_id)
        .iter()
        .copied()
        .find(|id| graph.symbols[id].name == from)
    else {
        plan.refused = Some(format!("{file} does not export {from}"));
        return Ok(plan);
    };
    if report.is_convex_function(symbol_id) {
        plan.refused = Some("renaming a Convex function changes its api path; rename it by hand".into());
        return Ok(plan);
    }
    if !is_identifier(to) {
        plan.refused = Some(format!("{to} is not an identifier"));
        return Ok(plan);
    }

    let mut files: Vec<(String, RenameRole)> = vec![(file.to_string(), RenameRole::Declaration)];
    for edge in &graph.edges {
        if edge.kind == ImportKind::Reexport
            && edge.target_file_id == Some(file_id)
            && edge.bindings.iter().any(|binding| binding.imported == from)
        {
            files.push((edge.source_path.clone(), RenameRole::Importer));
        }
    }
    for row in report.external_users(symbol_id) {
        files.push((graph.path(row.file_id).to_string(), RenameRole::Importer));
    }
    files.sort();
    files.dedup();

    for (path, role) in files {
        let Ok(source) = std::fs::read_to_string(root.join(&path)) else {
            continue;
        };
        let ext = path.rsplit('.').next().unwrap_or("ts");
        let edits = rename_edits(&source, ext, from, to, role);
        if !edits.is_empty() {
            plan.edits.insert(path, edits);
        }
    }
    Ok(plan)
}

/// Apply a rename, run the check, revert every file if it fails.
pub fn apply_rename(root: &Path, plan: &RenamePlan, check: Option<&str>) -> io::Result<String> {
    if plan.refused.is_some() {
        return Ok("Nothing applied.".into());
    }
    let Some(check) = check else {
        return Err(io::Error::other(
            "--apply needs a check command (--check \"<cmd>\" or [fix] check in .prunerc)",
        ));
    };
    let check = Some(check);
    let mut originals: Vec<(PathBuf, String)> = Vec::new();
    for (file, edits) in &plan.edits {
        let path = root.join(file);
        let source = std::fs::read_to_string(&path)?;
        let mut sorted = edits.clone();
        sorted.sort_by_key(|(span, _)| std::cmp::Reverse(span.start));
        let mut text = source.clone();
        for (span, replacement) in sorted {
            text.replace_range(span.start as usize..span.end as usize, &replacement);
        }
        std::fs::write(&path, text)?;
        originals.push((path, source));
    }
    if let Some(command) = check {
        let outcome = run_check(root, command)?;
        if !outcome.passed {
            for (path, source) in &originals {
                std::fs::write(path, source)?;
            }
            return Ok(format!("Reverted: `{command}` failed.\n{}", outcome.tail));
        }
        return Ok(format!("Renamed in {} file(s); `{command}` passed.", originals.len()));
    }
    Ok(format!("Renamed in {} file(s).", originals.len()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum RenameRole {
    Declaration,
    Importer,
}

fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|ch| ch.is_alphabetic() || ch == '_' || ch == '$')
        && chars.all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '$')
}

/// Spans to rewrite in one file.
fn rename_edits(source: &str, ext: &str, from: &str, to: &str, role: RenameRole) -> Vec<(Span, String)> {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(Path::new(&format!("file.{ext}"))).unwrap_or_default();
    let parsed = OxcParser::new(&allocator, source, source_type).parse();
    let program = parsed.program;
    let semantic = SemanticBuilder::new().with_build_nodes(true).build(&program);
    let scoping = semantic.semantic.scoping();
    let nodes = semantic.semantic.nodes();
    let mut edits: Vec<(Span, String)> = Vec::new();

    let rename_binding = |symbol_id: oxc_semantic::SymbolId, edits: &mut Vec<(Span, String)>| {
        edits.push((scoping.symbol_span(symbol_id), to.to_string()));
        for reference in scoping.get_resolved_references(symbol_id) {
            let node = reference.node_id();
            let span = nodes.kind(node).span();
            // `{ from }` shorthand in an object literal keeps its key.
            if let AstKind::ObjectProperty(property) = nodes.parent_kind(node) {
                if property.shorthand {
                    edits.push((span, format!("{from}: {to}")));
                    continue;
                }
            }
            edits.push((span, to.to_string()));
        }
    };

    if role == RenameRole::Declaration {
        if let Some(symbol_id) = scoping.get_root_binding(from.into()) {
            rename_binding(symbol_id, &mut edits);
        }
        return dedupe(edits);
    }

    for statement in &program.body {
        match statement {
            Statement::ImportDeclaration(import) => {
                for spec in import.specifiers.iter().flatten() {
                    match spec {
                        ImportDeclarationSpecifier::ImportSpecifier(spec)
                            if export_name(&spec.imported) == from =>
                        {
                            if spec.local.name.as_str() == from {
                                if spec.imported.span() != spec.local.span {
                                    edits.push((spec.imported.span(), to.to_string()));
                                }
                                if let Some(symbol_id) = spec.local.symbol_id.get() {
                                    rename_binding(symbol_id, &mut edits);
                                }
                            } else {
                                edits.push((spec.imported.span(), to.to_string()));
                            }
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(spec) => {
                            let Some(symbol_id) = spec.local.symbol_id.get() else {
                                continue;
                            };
                            for reference in scoping.get_resolved_references(symbol_id) {
                                if let AstKind::StaticMemberExpression(member) =
                                    nodes.parent_kind(reference.node_id())
                                {
                                    if member.property.name.as_str() == from {
                                        edits.push((member.property.span, to.to_string()));
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Statement::ExportFromDeclaration(export) => {
                for spec in &export.specifiers {
                    if export_name(&spec.local) == from {
                        if spec.local.span() == spec.exported.span() {
                            edits.push((spec.local.span(), to.to_string()));
                        } else {
                            edits.push((spec.local.span(), to.to_string()));
                            if export_name(&spec.exported) == from {
                                edits.push((spec.exported.span(), to.to_string()));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    dedupe(edits)
}

fn dedupe(mut edits: Vec<(Span, String)>) -> Vec<(Span, String)> {
    edits.sort_by_key(|(span, _)| (span.start, span.end));
    edits.dedup_by_key(|(span, _)| span.start);
    edits
}

fn export_name(name: &ModuleExportName<'_>) -> String {
    match name {
        ModuleExportName::IdentifierName(id) => id.name.to_string(),
        ModuleExportName::IdentifierReference(id) => id.name.to_string(),
        ModuleExportName::StringLiteral(lit) => lit.value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unexport_plain_declarations_only() {
        let source = "export function a() {}\nexport const b = 1;\nconst c = 2;\nexport { c };\nexport default function d() {}\n";
        let reexported: HashSet<String> = ["b".to_string()].into_iter().collect();
        let (out, done, skipped) = unexport_source(
            source,
            &[("a".into(), 1), ("b".into(), 2), ("c".into(), 3)],
            &reexported,
        );
        assert_eq!(done, vec!["a"]);
        assert!(out.starts_with("function a() {}\nexport const b"));
        assert_eq!(skipped.len(), 2);
    }

    #[test]
    fn rename_edits_cover_declaration_imports_barrels_and_namespaces() {
        let decl = "export function old() { return old.name; }\nconst x = { old };\n";
        let edits = rename_edits(decl, "ts", "old", "fresh", RenameRole::Declaration);
        assert_eq!(edits.len(), 3, "{edits:?}");
        assert!(edits.iter().any(|(_, text)| text == "old: fresh"));

        let importer = "import { old } from './a';\nimport { old as alias } from './b';\nimport * as ns from './c';\nold(); alias(); ns.old();\n";
        let edits = rename_edits(importer, "ts", "old", "fresh", RenameRole::Importer);
        // import specifier (shorthand: one span), call site, aliased imported name, ns member
        assert_eq!(edits.len(), 4, "{edits:?}");

        let barrel = "export { old } from './a';\nexport { old as other } from './b';\n";
        let edits = rename_edits(barrel, "ts", "old", "fresh", RenameRole::Importer);
        assert_eq!(edits.len(), 2, "{edits:?}");
    }
}
