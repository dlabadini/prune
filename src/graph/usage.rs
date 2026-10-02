//! Classify every export by who reaches it: product code, tests, its own file, or nobody.

use super::ModuleGraph;
use crate::db::{Database, DbError, FileRole, RefRow};
use crate::entries::CONVEX_FUNCTION_TAG;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// How an export is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Usage {
    /// Imported or referenced from product code, or consumed by a framework/tool.
    Used,
    /// Only tests import it.
    TestOnly,
    /// Only its own file uses it: the `export` keyword is unnecessary.
    SameFileOnly,
    /// Nothing uses it.
    Unused,
    /// Its file is not reachable from any entry.
    Unreachable,
}

impl Usage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Usage::Used => "used",
            Usage::TestOnly => "test_only",
            Usage::SameFileOnly => "same_file_only",
            Usage::Unused => "unused",
            Usage::Unreachable => "unreachable",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Usage::Used => "used",
            Usage::TestOnly => "used only by tests",
            Usage::SameFileOnly => "used only in its own file",
            Usage::Unused => "unused",
            Usage::Unreachable => "in an unreachable file",
        }
    }
}

/// Whether an entry reaches a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reach {
    /// Reached from a product entry.
    Product,
    /// Reached only from tests (test support code).
    TestOnly,
    /// Not reached at all.
    Unreachable,
}

pub struct UsageReport {
    pub graph: ModuleGraph,
    product: HashSet<i64>,
    tests: HashSet<i64>,
    /// Uses per symbol (import, framework reference, or same-file use).
    pub users: HashMap<i64, Vec<RefRow>>,
    /// Exports consumed outside the graph, with why.
    pub roots: HashMap<i64, String>,
    pub tags: HashMap<i64, Vec<String>>,
    /// Entry files, with why.
    pub entries: HashMap<i64, Vec<String>>,
    usage: HashMap<i64, Usage>,
}

impl UsageReport {
    pub fn load(db: &Database) -> Result<Self, DbError> {
        let graph = ModuleGraph::load(db)?;
        let mut entries: HashMap<i64, Vec<String>> = HashMap::new();
        for (file_id, reason) in db.entries()? {
            entries.entry(file_id).or_default().push(reason);
        }
        let roots: HashMap<i64, String> = db.symbol_roots()?.into_iter().collect();
        let mut tags: HashMap<i64, Vec<String>> = HashMap::new();
        for (symbol_id, tag) in db.symbol_tags()? {
            tags.entry(symbol_id).or_default().push(tag);
        }
        let mut users: HashMap<i64, Vec<RefRow>> = HashMap::new();
        for row in db.all_refs()? {
            users.entry(row.symbol_id).or_default().push(row);
        }

        let product_roots: Vec<i64> = entries
            .keys()
            .copied()
            .filter(|id| graph.role(*id) == FileRole::Source)
            .collect();
        let product = graph.reachable(product_roots, |_| true);
        let test_roots: Vec<i64> = graph
            .files
            .values()
            .filter(|file| file.role == FileRole::Test)
            .map(|file| file.id)
            .collect();
        let tests = graph.reachable(test_roots, |_| true);

        let mut report = Self {
            graph,
            product,
            tests,
            users,
            roots,
            tags,
            entries,
            usage: HashMap::new(),
        };
        report.classify();
        Ok(report)
    }

    fn classify(&mut self) {
        let mut usage = HashMap::new();
        for symbol in self.graph.symbols.values().filter(|symbol| symbol.is_exported) {
            usage.insert(symbol.id, self.classify_symbol(symbol.id, symbol.file_id));
        }
        self.usage = usage;
    }

    fn classify_symbol(&self, symbol_id: i64, file_id: i64) -> Usage {
        match self.reach(file_id) {
            Reach::Unreachable => return Usage::Unreachable,
            Reach::TestOnly => return Usage::TestOnly,
            Reach::Product => {}
        }
        if self.roots.contains_key(&symbol_id) {
            return Usage::Used;
        }
        let rows = self.users.get(&symbol_id).map(Vec::as_slice).unwrap_or(&[]);
        let mut by_tests = false;
        let mut in_file = false;
        for row in rows {
            if row.file_id == file_id {
                // `internal.self.fn` from the same module is a real (scheduled/run) use.
                if row.derived && self.is_convex_function(symbol_id) {
                    return Usage::Used;
                }
                in_file = true;
                continue;
            }
            match self.reach_of_user(row.file_id) {
                Reach::Product => return Usage::Used,
                Reach::TestOnly => by_tests = true,
                // A use from code nothing reaches does not keep a symbol alive.
                Reach::Unreachable => {}
            }
        }
        if by_tests {
            Usage::TestOnly
        } else if in_file {
            Usage::SameFileOnly
        } else {
            Usage::Unused
        }
    }

    fn reach_of_user(&self, file_id: i64) -> Reach {
        if self.graph.role(file_id) == FileRole::Test {
            return Reach::TestOnly;
        }
        self.reach(file_id)
    }

    /// Whether an entry reaches a file.
    pub fn reach(&self, file_id: i64) -> Reach {
        if self.product.contains(&file_id) {
            Reach::Product
        } else if self.tests.contains(&file_id) {
            Reach::TestOnly
        } else {
            Reach::Unreachable
        }
    }

    pub fn usage(&self, symbol_id: i64) -> Option<Usage> {
        self.usage.get(&symbol_id).copied()
    }

    pub fn is_convex_function(&self, symbol_id: i64) -> bool {
        self.tags
            .get(&symbol_id)
            .is_some_and(|tags| tags.iter().any(|tag| tag == CONVEX_FUNCTION_TAG))
    }

    /// Uses of a symbol from other files.
    pub fn external_users(&self, symbol_id: i64) -> Vec<RefRow> {
        let Some(symbol) = self.graph.symbols.get(&symbol_id) else {
            return Vec::new();
        };
        self.users
            .get(&symbol_id)
            .map(|rows| {
                rows.iter()
                    .filter(|row| row.file_id != symbol.file_id)
                    .copied()
                    .collect()
            })
            .unwrap_or_default()
    }
}
