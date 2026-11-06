//! In-memory module graph: names resolved through barrels, reachability, value vs type edges.
//!
//! Loaded from the index once per detector run. Every query here is pure over that snapshot.

pub mod usage;

use crate::analyzer::parser::ImportKind;
use crate::db::{Database, DbError, FileRole, FileRow, ImportEdge, Symbol, SymbolKind};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};

/// A resolved use of a symbol from another module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Use {
    pub symbol_id: i64,
    pub file_id: i64,
    pub line: usize,
    /// The use only needs the symbol's type (erased at runtime).
    pub type_only: bool,
}

pub struct ModuleGraph {
    pub files: HashMap<i64, FileRow>,
    pub symbols: HashMap<i64, Symbol>,
    /// Exported symbols per file.
    exports: HashMap<i64, Vec<i64>>,
    /// Resolved edges (target known).
    pub edges: Vec<ImportEdge>,
    /// Edge indexes leaving each file.
    out: HashMap<i64, Vec<usize>>,
    all_exports_cache: RefCell<HashMap<i64, Vec<i64>>>,
}

impl ModuleGraph {
    pub fn load(db: &Database) -> Result<Self, DbError> {
        let files: HashMap<i64, FileRow> = db
            .file_rows()?
            .into_iter()
            .map(|row| (row.id, row))
            .collect();
        let mut symbols = HashMap::new();
        let mut exports: HashMap<i64, Vec<i64>> = HashMap::new();
        for symbol in db.all_symbols()? {
            if symbol.is_exported {
                exports.entry(symbol.file_id).or_default().push(symbol.id);
            }
            symbols.insert(symbol.id, symbol);
        }
        let edges: Vec<ImportEdge> = db
            .import_edges()?
            .into_iter()
            .filter(|edge| edge.target_file_id.is_some())
            .collect();
        Ok(Self::from_parts(files, symbols, exports, edges))
    }

    fn from_parts(
        files: HashMap<i64, FileRow>,
        symbols: HashMap<i64, Symbol>,
        exports: HashMap<i64, Vec<i64>>,
        edges: Vec<ImportEdge>,
    ) -> Self {
        let mut out: HashMap<i64, Vec<usize>> = HashMap::new();
        for (index, edge) in edges.iter().enumerate() {
            out.entry(edge.source_file_id).or_default().push(index);
        }
        Self {
            files,
            symbols,
            exports,
            edges,
            out,
            all_exports_cache: RefCell::new(HashMap::new()),
        }
    }

    pub fn path(&self, file_id: i64) -> &str {
        self.files
            .get(&file_id)
            .map(|file| file.path.as_str())
            .unwrap_or("")
    }

    pub fn role(&self, file_id: i64) -> FileRole {
        self.files
            .get(&file_id)
            .map(|file| file.role)
            .unwrap_or(FileRole::Source)
    }

    pub fn file_id(&self, path: &str) -> Option<i64> {
        self.files
            .values()
            .find(|file| file.path == path)
            .map(|file| file.id)
    }

    pub fn exported(&self, file_id: i64) -> &[i64] {
        self.exports.get(&file_id).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn edges_from(&self, file_id: i64) -> impl Iterator<Item = &ImportEdge> {
        self.out
            .get(&file_id)
            .into_iter()
            .flatten()
            .map(|index| &self.edges[*index])
    }

    /// The symbols a name exported by `file_id` stands for, following re-exports and `export *`.
    pub fn resolve_export(&self, file_id: i64, name: &str) -> Vec<i64> {
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        self.resolve_into(file_id, name, &mut seen, &mut found);
        found.sort_unstable();
        found.dedup();
        found
    }

    fn resolve_into(
        &self,
        file_id: i64,
        name: &str,
        seen: &mut HashSet<(i64, String)>,
        found: &mut Vec<i64>,
    ) {
        if !seen.insert((file_id, name.to_string())) {
            return;
        }
        let before = found.len();
        for symbol_id in self.exported(file_id) {
            let symbol = &self.symbols[symbol_id];
            if symbol.name == name || (name == "default" && symbol.is_default) {
                found.push(*symbol_id);
            }
        }
        if found.len() > before {
            return;
        }
        for edge in self.edges_from(file_id) {
            if edge.kind != ImportKind::Reexport {
                continue;
            }
            let Some(target) = edge.target_file_id else {
                continue;
            };
            for binding in &edge.bindings {
                if binding.local != name {
                    continue;
                }
                if binding.imported == "*" {
                    found.extend(self.all_exports(target));
                } else {
                    self.resolve_into(target, &binding.imported, seen, found);
                }
            }
        }
        if found.len() > before || name == "default" {
            return;
        }
        for edge in self.edges_from(file_id) {
            if edge.kind == ImportKind::Reexport && edge.star {
                if let Some(target) = edge.target_file_id {
                    self.resolve_into(target, name, seen, found);
                }
            }
        }
    }

    /// Every symbol `file_id` exports, including what it re-exports.
    pub fn all_exports(&self, file_id: i64) -> Vec<i64> {
        if let Some(cached) = self.all_exports_cache.borrow().get(&file_id) {
            return cached.clone();
        }
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        self.all_exports_into(file_id, &mut seen, &mut found);
        found.sort_unstable();
        found.dedup();
        self.all_exports_cache
            .borrow_mut()
            .insert(file_id, found.clone());
        found
    }

    fn all_exports_into(&self, file_id: i64, seen: &mut HashSet<i64>, found: &mut Vec<i64>) {
        if !seen.insert(file_id) {
            return;
        }
        found.extend_from_slice(self.exported(file_id));
        for edge in self.edges_from(file_id) {
            if edge.kind != ImportKind::Reexport {
                continue;
            }
            let Some(target) = edge.target_file_id else {
                continue;
            };
            if edge.star {
                // `export *` never forwards `default`.
                let mut inner = Vec::new();
                self.all_exports_into(target, seen, &mut inner);
                found.extend(
                    inner
                        .into_iter()
                        .filter(|id| !self.symbols[id].is_default || self.symbols[id].name != "default"),
                );
                continue;
            }
            for binding in &edge.bindings {
                if binding.imported == "*" {
                    let mut inner = Vec::new();
                    self.all_exports_into(target, seen, &mut inner);
                    found.extend(inner);
                } else {
                    let mut name_seen = HashSet::new();
                    self.resolve_into(target, &binding.imported, &mut name_seen, found);
                }
            }
        }
    }

    /// Uses created by imports (not re-exports: those only forward a name).
    pub fn import_uses(&self) -> Vec<Use> {
        let mut uses = Vec::new();
        for edge in &self.edges {
            if edge.kind == ImportKind::Reexport {
                continue;
            }
            let Some(target) = edge.target_file_id else {
                continue;
            };
            for binding in &edge.bindings {
                if binding.imported == "*" {
                    continue;
                }
                for symbol_id in self.resolve_export(target, &binding.imported) {
                    uses.push(Use {
                        symbol_id,
                        file_id: edge.source_file_id,
                        line: edge.line,
                        type_only: binding.is_type || edge.is_type_only,
                    });
                }
            }
            if edge.namespace {
                let symbols = match &edge.namespace_members {
                    Some(members) => members
                        .iter()
                        .flat_map(|member| self.resolve_export(target, member))
                        .collect(),
                    None => self.all_exports(target),
                };
                for symbol_id in symbols {
                    uses.push(Use {
                        symbol_id,
                        file_id: edge.source_file_id,
                        line: edge.line,
                        type_only: edge.is_type_only,
                    });
                }
            }
        }
        uses.sort_by_key(|item| (item.symbol_id, item.file_id, item.line));
        uses.dedup();
        uses
    }

    /// Files reachable from `roots` following edges accepted by `follow`.
    pub fn reachable(
        &self,
        roots: impl IntoIterator<Item = i64>,
        follow: impl Fn(&ImportEdge) -> bool,
    ) -> HashSet<i64> {
        let mut seen: HashSet<i64> = HashSet::new();
        let mut queue: VecDeque<i64> = VecDeque::new();
        for root in roots {
            if seen.insert(root) {
                queue.push_back(root);
            }
        }
        while let Some(file_id) = queue.pop_front() {
            for edge in self.edges_from(file_id) {
                if !follow(edge) {
                    continue;
                }
                if let Some(target) = edge.target_file_id {
                    if seen.insert(target) {
                        queue.push_back(target);
                    }
                }
            }
        }
        seen
    }

    /// Whether an edge disappears at runtime: declared `type`, used only as a type, or every
    /// name it brings in resolves to an interface or type alias.
    pub fn edge_is_type_only(&self, edge: &ImportEdge) -> bool {
        if edge.is_type_only {
            return true;
        }
        if matches!(edge.kind, ImportKind::SideEffect | ImportKind::Path) {
            return false;
        }
        if edge.namespace || edge.star || edge.bindings.is_empty() {
            return false;
        }
        let Some(target) = edge.target_file_id else {
            return false;
        };
        edge.bindings.iter().all(|binding| {
            if binding.is_type {
                return true;
            }
            if binding.imported == "*" {
                return false;
            }
            let resolved = self.resolve_export(target, &binding.imported);
            !resolved.is_empty()
                && resolved.iter().all(|id| {
                    matches!(
                        self.symbols[id].kind,
                        SymbolKind::Interface | SymbolKind::TypeAlias
                    )
                })
        })
    }

    /// Whether an edge orders module initialization (a runtime value edge).
    pub fn edge_is_value(&self, edge: &ImportEdge) -> bool {
        !matches!(edge.kind, ImportKind::Dynamic | ImportKind::Path) && !self.edge_is_type_only(edge)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::parser::ImportBinding;

    fn file(id: i64, path: &str) -> (i64, FileRow) {
        (
            id,
            FileRow {
                id,
                path: path.into(),
                role: FileRole::Source,
                line_count: 1,
                executable: false,
            },
        )
    }

    fn symbol(id: i64, file_id: i64, name: &str, kind: SymbolKind) -> Symbol {
        Symbol {
            id,
            file_id,
            name: name.into(),
            kind,
            start_line: 1,
            end_line: 1,
            is_exported: true,
            is_default: false,
            source_text: String::new(),
            callee: None,
        }
    }

    fn edge(source: i64, target: i64, kind: ImportKind, bindings: &[(&str, &str)]) -> ImportEdge {
        ImportEdge {
            id: 0,
            source_file_id: source,
            source_path: String::new(),
            target_file_id: Some(target),
            target_path: None,
            specifier: String::new(),
            kind,
            bindings: bindings
                .iter()
                .map(|(imported, local)| ImportBinding {
                    imported: (*imported).into(),
                    local: (*local).into(),
                    is_type: false,
                })
                .collect(),
            namespace: false,
            star: false,
            namespace_members: None,
            is_type_only: false,
            line: 1,
        }
    }

    #[test]
    fn resolves_through_named_and_star_barrels() {
        // 1 app imports { b } from 2 index; 2: export { a as b } from 3; export * from 4
        let files = [file(1, "app.ts"), file(2, "index.ts"), file(3, "a.ts"), file(4, "c.ts")]
            .into_iter()
            .collect();
        let symbols: HashMap<i64, Symbol> = [
            (10, symbol(10, 3, "a", SymbolKind::Function)),
            (11, symbol(11, 4, "c", SymbolKind::Interface)),
            (12, symbol(12, 4, "unused", SymbolKind::Function)),
        ]
        .into_iter()
        .collect();
        let exports = [(3, vec![10]), (4, vec![11, 12])].into_iter().collect();
        let mut star = edge(2, 4, ImportKind::Reexport, &[]);
        star.star = true;
        let edges = vec![
            edge(1, 2, ImportKind::Static, &[("b", "b"), ("c", "c")]),
            edge(2, 3, ImportKind::Reexport, &[("a", "b")]),
            star,
        ];
        let graph = ModuleGraph::from_parts(files, symbols, exports, edges);
        assert_eq!(graph.resolve_export(2, "b"), vec![10]);
        assert_eq!(graph.resolve_export(2, "c"), vec![11]);
        let used: HashSet<i64> = graph.import_uses().iter().map(|u| u.symbol_id).collect();
        assert!(used.contains(&10) && used.contains(&11) && !used.contains(&12));
        assert_eq!(graph.all_exports(2), vec![10, 11, 12]);
        // Importing only an interface is a type edge.
        let type_edge = edge(1, 4, ImportKind::Static, &[("c", "c")]);
        assert!(graph.edge_is_type_only(&type_edge));
        assert!(!graph.edge_is_type_only(&graph.edges[0]));
    }

    #[test]
    fn reexport_cycles_terminate() {
        let files = [file(1, "a.ts"), file(2, "b.ts")].into_iter().collect();
        let mut ab = edge(1, 2, ImportKind::Reexport, &[]);
        ab.star = true;
        let mut ba = edge(2, 1, ImportKind::Reexport, &[]);
        ba.star = true;
        let graph = ModuleGraph::from_parts(files, HashMap::new(), HashMap::new(), vec![ab, ba]);
        assert!(graph.resolve_export(1, "x").is_empty());
        assert!(graph.all_exports(1).is_empty());
    }
}
