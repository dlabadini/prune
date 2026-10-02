//! Entry discovery: package manifests, scripts, config files, and framework adapters.
//!
//! An entry is a file something outside the module graph loads (a runtime, a bundler, a
//! framework, a CLI). Roots are exports that something outside the graph consumes.

use crate::analyzer::parser::{is_function_name_string, NameRefKind};
use crate::config::{glob_match, Config, ConvexConfig};
use crate::db::{FileRole, StoredNameRef};
use crate::graph::ModuleGraph;
use std::collections::{HashMap, HashSet};
use std::path::Path;

pub const CONVEX_FUNCTION_TAG: &str = "convex-function";

const SOURCE_EXTENSIONS: &[&str] = &["ts", "tsx", "js", "jsx", "mts", "mjs", "cts", "cjs"];

/// Everything discovered about how the program starts.
#[derive(Debug, Default)]
pub struct EntryPlan {
    /// Files loaded from outside the graph.
    pub entries: Vec<(i64, String)>,
    /// Files whose every export is consumed from outside the graph.
    pub root_files: Vec<(i64, String)>,
    /// Individual exports consumed from outside the graph.
    pub root_symbols: Vec<(i64, String)>,
    /// Symbols tagged by adapters (`convex-function`).
    pub tags: Vec<(i64, &'static str)>,
    /// Uses resolved by adapters: (symbol, referencing file, line).
    pub uses: Vec<(i64, i64, usize)>,
}

/// Lookup from repo-relative path to file id.
struct Paths<'a> {
    by_path: HashMap<&'a str, i64>,
}

impl<'a> Paths<'a> {
    fn new(graph: &'a ModuleGraph) -> Self {
        Self {
            by_path: graph
                .files
                .values()
                .map(|file| (file.path.as_str(), file.id))
                .collect(),
        }
    }

    fn get(&self, path: &str) -> Option<i64> {
        self.by_path.get(path).copied()
    }

    /// `dir/x` → `dir/x`, `dir/x.ts`, `dir/x/index.ts`, also `dist/` → `src/` and `.js` → `.ts`.
    fn resolve_module(&self, path: &str) -> Option<i64> {
        let path = path.trim_start_matches("./");
        for candidate in module_candidates(path) {
            if let Some(id) = self.get(&candidate) {
                return Some(id);
            }
        }
        for (from, to) in [("dist/", "src/"), ("build/", "src/"), ("lib/", "src/"), ("out/", "src/")] {
            if path.contains(from) {
                let swapped = path.replacen(from, to, 1);
                for candidate in module_candidates(&swapped) {
                    if let Some(id) = self.get(&candidate) {
                        return Some(id);
                    }
                }
            }
        }
        None
    }
}

fn module_candidates(path: &str) -> Vec<String> {
    let mut out = vec![path.to_string()];
    let stem = strip_source_ext(path);
    for ext in SOURCE_EXTENSIONS {
        out.push(format!("{stem}.{ext}"));
    }
    for ext in SOURCE_EXTENSIONS {
        out.push(format!("{stem}/index.{ext}"));
    }
    if let Some(base) = path.strip_suffix(".d.ts") {
        out.push(format!("{base}.ts"));
    }
    out
}

fn strip_source_ext(path: &str) -> &str {
    for ext in SOURCE_EXTENSIONS {
        if let Some(stem) = path.strip_suffix(&format!(".{ext}")) {
            return stem;
        }
    }
    path
}

/// Join `base` and a relative `path`, folding `.` and `..` textually.
pub fn join_relative(base: &str, path: &str) -> String {
    let mut parts: Vec<&str> = base.split('/').filter(|part| !part.is_empty()).collect();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("")
}

fn file_stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    strip_source_ext(name)
}

/// Discover entries and roots for the indexed project at `root`.
pub fn discover(
    root: &Path,
    config: &Config,
    graph: &ModuleGraph,
    name_refs: &[StoredNameRef],
) -> EntryPlan {
    let paths = Paths::new(graph);
    let mut plan = EntryPlan::default();

    discover_packages(root, config, graph, &paths, &mut plan);
    discover_tool_references(root, &paths, &mut plan);
    discover_config_files(config, graph, &mut plan);
    if config.frameworks.next.enabled {
        discover_next(root, config, graph, &paths, &mut plan);
    }
    if config.frameworks.convex.enabled {
        discover_convex(root, &config.frameworks.convex, graph, &paths, name_refs, &mut plan);
    }

    plan.entries.sort();
    plan.entries.dedup();
    plan.root_files.sort();
    plan.root_files.dedup();
    plan
}

fn source_files(graph: &ModuleGraph) -> impl Iterator<Item = (i64, &str)> {
    graph
        .files
        .values()
        .filter(|file| file.role == FileRole::Source)
        .map(|file| (file.id, file.path.as_str()))
}

fn discover_config_files(config: &Config, graph: &ModuleGraph, plan: &mut EntryPlan) {
    for file in graph.files.values().filter(|file| file.role == FileRole::Source) {
        if file.executable {
            plan.entries.push((file.id, "executable script".into()));
            plan.root_files.push((file.id, "executable script".into()));
        }
        if file.path.ends_with(".d.ts") {
            plan.entries.push((file.id, "ambient declarations".into()));
            plan.root_files.push((file.id, "ambient declarations".into()));
        }
    }
    for (id, path) in source_files(graph) {
        let name = path.rsplit('/').next().unwrap_or(path);
        let stem = strip_source_ext(name);
        let is_tool_config = stem.ends_with(".config")
            || stem.contains(".config.")
            || (name.starts_with('.') && stem.ends_with("rc"));
        if is_tool_config {
            plan.entries.push((id, "config file".into()));
            plan.root_files.push((id, "config file".into()));
        }
        if config
            .entries
            .files
            .iter()
            .any(|pattern| glob_match(pattern, path))
        {
            plan.entries.push((id, "configured entry".into()));
            plan.root_files.push((id, "configured entry".into()));
        }
    }
}

fn discover_packages(
    root: &Path,
    config: &Config,
    graph: &ModuleGraph,
    paths: &Paths<'_>,
    plan: &mut EntryPlan,
) {
    let walker = ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .filter_entry(|entry| entry.file_name() != "node_modules")
        .build();
    for entry in walker.flatten() {
        if entry.file_name() != "package.json" || !entry.path().is_file() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let dir = entry
            .path()
            .parent()
            .and_then(|parent| parent.strip_prefix(root).ok())
            .map(|rel| rel.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        let private = manifest
            .get("private")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let public_api = !private || config.entries.private_package_exports_are_api;

        let mut targets = Vec::new();
        for field in ["main", "module", "types", "typings", "browser", "source"] {
            if let Some(value) = manifest.get(field).and_then(|value| value.as_str()) {
                targets.push(value.to_string());
            }
        }
        if let Some(bin) = manifest.get("bin") {
            collect_strings(bin, &mut targets);
        }
        if let Some(exports) = manifest.get("exports") {
            collect_strings(exports, &mut targets);
        }
        for target in targets {
            if target.contains('*') {
                let pattern = join_relative(&dir, &target);
                for (id, path) in source_files(graph) {
                    if wildcard_match(&pattern, path) {
                        push_package_entry(plan, id, public_api);
                    }
                }
                continue;
            }
            if let Some(id) = paths.resolve_module(&join_relative(&dir, &target)) {
                push_package_entry(plan, id, public_api);
            }
        }

        if let Some(scripts) = manifest.get("scripts").and_then(|value| value.as_object()) {
            for command in scripts.values().filter_map(|value| value.as_str()) {
                for token in script_tokens(command) {
                    if is_function_name_string(&token) {
                        resolve_function_string_root(&token, graph, &convex_dirs_for(root, &config.frameworks.convex), plan);
                        continue;
                    }
                    if looks_like_path(&token) {
                        if let Some(id) = paths.resolve_module(&join_relative(&dir, &token)) {
                            plan.entries.push((id, "package script".into()));
                            plan.root_files.push((id, "package script".into()));
                        }
                    }
                }
            }
        }
    }
}

/// Paths named in root-level tool configs (`bunfig.toml` preloads, `vercel.json`, CI yaml).
fn discover_tool_references(root: &Path, paths: &Paths<'_>, plan: &mut EntryPlan) {
    let Ok(dir) = std::fs::read_dir(root) else {
        return;
    };
    for entry in dir.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_config_text = [".toml", ".json", ".yaml", ".yml", ".jsonc"]
            .iter()
            .any(|ext| name.ends_with(ext))
            && name != "package.json"
            && !name.starts_with("tsconfig")
            && !name.starts_with("jsconfig")
            && !name.contains("lock");
        if !is_config_text || !entry.path().is_file() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        if text.len() > 512 * 1024 {
            continue;
        }
        for token in text.split(|ch: char| {
            ch.is_whitespace() || matches!(ch, '"' | '\'' | ',' | '[' | ']' | '=' | '`' | '(' | ')')
        }) {
            if looks_like_path(token) {
                if let Some(id) = paths.resolve_module(&join_relative("", token)) {
                    plan.entries.push((id, format!("referenced by {name}")));
                }
            }
        }
    }
}

fn push_package_entry(plan: &mut EntryPlan, id: i64, public_api: bool) {
    plan.entries.push((id, "package entry".into()));
    if public_api {
        plan.root_files.push((id, "public package entry".into()));
    }
}

fn collect_strings(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(text) => out.push(text.clone()),
        serde_json::Value::Array(items) => items.iter().for_each(|item| collect_strings(item, out)),
        serde_json::Value::Object(map) => map.values().for_each(|item| collect_strings(item, out)),
        _ => {}
    }
}

/// `src/*.ts` matches `src/a.ts`; one `*` may span directories.
fn wildcard_match(pattern: &str, path: &str) -> bool {
    match pattern.split_once('*') {
        Some((prefix, suffix)) => {
            path.starts_with(prefix)
                && path.len() >= prefix.len() + suffix.len()
                && (path.ends_with(suffix)
                    || strip_source_ext(path).ends_with(strip_source_ext(suffix)))
        }
        None => pattern == path,
    }
}

fn script_tokens(command: &str) -> Vec<String> {
    command
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '(' | ')'))
        .map(|token| {
            token
                .trim_matches(|ch| matches!(ch, '"' | '\'' | '`'))
                .split('=')
                .next_back()
                .unwrap_or("")
                .to_string()
        })
        .filter(|token| !token.is_empty() && !token.starts_with('-'))
        .collect()
}

fn looks_like_path(token: &str) -> bool {
    let ext = token.rsplit('.').next().unwrap_or("");
    token.contains('/') && SOURCE_EXTENSIONS.contains(&ext)
}

fn discover_next(
    root: &Path,
    config: &Config,
    graph: &ModuleGraph,
    paths: &Paths<'_>,
    plan: &mut EntryPlan,
) {
    let next = &config.frameworks.next;
    let mut roots: Vec<String> = next.roots.clone();
    if roots.is_empty() {
        for (_, path) in source_files(graph) {
            if file_stem(path) == "next.config" {
                roots.push(parent_dir(path).to_string());
            }
        }
        for ext in ["js", "mjs", "ts", "mts", "cjs"] {
            if root.join(format!("next.config.{ext}")).is_file() && !roots.iter().any(|r| r.is_empty()) {
                roots.push(String::new());
            }
        }
    }
    let app_entries: HashSet<&str> = next.app_entries.iter().map(String::as_str).collect();
    for app_root in &roots {
        let base = |sub: &str| {
            if app_root.is_empty() {
                sub.to_string()
            } else {
                format!("{app_root}/{sub}")
            }
        };
        let app_dirs = [base("app/"), base("src/app/")];
        let page_dirs = [base("pages/"), base("src/pages/"), base("public/")];
        for (id, path) in source_files(graph) {
            let in_app = app_dirs.iter().any(|dir| path.starts_with(dir.as_str()));
            if in_app && app_entries.contains(file_stem(path)) {
                plan.entries.push((id, "next app route".into()));
                plan.root_files.push((id, "next app route".into()));
            }
            if page_dirs.iter().any(|dir| path.starts_with(dir.as_str())) {
                plan.entries.push((id, "next page".into()));
                plan.root_files.push((id, "next page".into()));
            }
        }
        for stem in &next.root_entries {
            for dir in [base(""), base("src/")] {
                if let Some(id) = paths.resolve_module(&format!("{dir}{stem}")) {
                    if file_stem(paths_path(graph, id)) == stem.as_str() {
                        plan.entries.push((id, "next root entry".into()));
                        plan.root_files.push((id, "next root entry".into()));
                    }
                }
            }
        }
    }
}

fn paths_path(graph: &ModuleGraph, id: i64) -> &str {
    graph.path(id)
}

/// Convex function directories, from config, `convex.json`, or `convex/`.
pub fn convex_dirs_for(root: &Path, convex: &ConvexConfig) -> Vec<String> {
    if !convex.dirs.is_empty() {
        return convex
            .dirs
            .iter()
            .map(|dir| dir.trim_end_matches('/').trim_start_matches("./").to_string())
            .collect();
    }
    if let Ok(text) = std::fs::read_to_string(root.join("convex.json")) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(dir) = value.get("functions").and_then(|v| v.as_str()) {
                return vec![dir.trim_end_matches('/').trim_start_matches("./").to_string()];
            }
        }
    }
    if root.join("convex").is_dir() {
        return vec!["convex".to_string()];
    }
    Vec::new()
}

fn discover_convex(
    root: &Path,
    convex: &ConvexConfig,
    graph: &ModuleGraph,
    paths: &Paths<'_>,
    name_refs: &[StoredNameRef],
    plan: &mut EntryPlan,
) {
    let dirs = convex_dirs_for(root, convex);
    if dirs.is_empty() {
        return;
    }
    // Directories Convex treats as a deployment root: each dir plus every component dir.
    let mut component_roots: HashSet<String> = dirs.iter().cloned().collect();
    for (_, path) in source_files(graph) {
        if file_stem(path) == "convex.config" && dirs.iter().any(|dir| in_dir(path, dir)) {
            component_roots.insert(parent_dir(path).to_string());
        }
    }
    let entry_modules: HashSet<&str> = convex.entry_modules.iter().map(String::as_str).collect();

    for (id, path) in source_files(graph) {
        if !dirs.iter().any(|dir| in_dir(path, dir)) || path.contains("/_generated/") {
            continue;
        }
        plan.entries.push((id, "convex module".into()));
        if entry_modules.contains(file_stem(path)) && component_roots.contains(parent_dir(path)) {
            plan.root_files.push((id, "convex entry module".into()));
        }
        for symbol_id in graph.exported(id) {
            let symbol = &graph.symbols[symbol_id];
            if symbol
                .callee
                .as_deref()
                .is_some_and(|callee| convex.is_function_builder(callee))
            {
                plan.tags.push((*symbol_id, CONVEX_FUNCTION_TAG));
            }
        }
    }

    for name_ref in name_refs {
        let targets = match name_ref.kind {
            NameRefKind::MemberChain => {
                let Some((root_name, chain)) = name_ref.value.split_once(':') else {
                    continue;
                };
                let referrer = graph.path(name_ref.file_id);
                let base = chain_base(&dirs, referrer, name_ref.specifier.as_deref());
                let segments: Vec<&str> = chain.split('.').collect();
                let (base, segments) = if root_name == "components" {
                    if segments.len() < 2 {
                        continue;
                    }
                    let component = segments[0];
                    let candidates = [format!("{base}/components/{component}"), format!("{base}/{component}")];
                    let Some(found) = candidates.into_iter().find(|candidate| has_files_under(graph, candidate)) else {
                        continue;
                    };
                    (found, segments[1..].to_vec())
                } else {
                    (base, segments)
                };
                resolve_chain(graph, paths, &base, &segments)
            }
            NameRefKind::FunctionString => dirs
                .iter()
                .flat_map(|dir| resolve_function_string(paths, graph, dir, &name_ref.value))
                .collect(),
        };
        for symbol_id in targets {
            plan.uses.push((symbol_id, name_ref.file_id, name_ref.line));
        }
    }
}

fn resolve_function_string_root(
    value: &str,
    graph: &ModuleGraph,
    dirs: &[String],
    plan: &mut EntryPlan,
) {
    let paths = Paths::new(graph);
    for dir in dirs {
        for symbol_id in resolve_function_string(&paths, graph, dir, value) {
            plan.root_symbols.push((symbol_id, "package script".into()));
        }
    }
}

fn resolve_function_string(paths: &Paths<'_>, graph: &ModuleGraph, dir: &str, value: &str) -> Vec<i64> {
    let Some((module, function)) = value.split_once(':') else {
        return Vec::new();
    };
    let module = strip_source_ext(module);
    match module_file(paths, &format!("{dir}/{module}")) {
        Some(file_id) => graph.resolve_export(file_id, function),
        None => Vec::new(),
    }
}

fn in_dir(path: &str, dir: &str) -> bool {
    dir.is_empty() || path.starts_with(&format!("{dir}/"))
}

fn has_files_under(graph: &ModuleGraph, dir: &str) -> bool {
    graph.files.values().any(|file| in_dir(&file.path, dir))
}

/// The Convex root a `api`/`internal` import points at (`../../_generated/api` → `convex`).
fn chain_base(dirs: &[String], referrer: &str, specifier: Option<&str>) -> String {
    if let Some(specifier) = specifier {
        if let Some((before, _)) = specifier.split_once("_generated") {
            if specifier.starts_with('.') {
                return join_relative(parent_dir(referrer), before);
            }
            let before = before.trim_end_matches('/');
            if let Some(dir) = dirs
                .iter()
                .find(|dir| before.ends_with(dir.as_str()) || before.ends_with(dir.rsplit('/').next().unwrap_or(dir)))
            {
                return dir.clone();
            }
        }
    }
    dirs.first().cloned().unwrap_or_default()
}

/// `[comms, threads, drafts, create]` under `convex` → the `create` export of
/// `convex/comms/threads/drafts.ts`. A chain that stops at a module or directory keeps every
/// export under it (`const t = internal.comms.threads; t.drafts.create`).
fn resolve_chain(graph: &ModuleGraph, paths: &Paths<'_>, base: &str, segments: &[&str]) -> Vec<i64> {
    for split in (1..=segments.len()).rev() {
        let module = format!("{base}/{}", segments[..split].join("/"));
        let Some(file_id) = module_file(paths, &module) else {
            continue;
        };
        return match segments.get(split) {
            Some(function) => graph.resolve_export(file_id, function),
            None => graph.exported(file_id).to_vec(),
        };
    }
    let dir = format!("{base}/{}", segments.join("/"));
    if segments.is_empty() {
        return Vec::new();
    }
    graph
        .files
        .values()
        .filter(|file| in_dir(&file.path, &dir))
        .flat_map(|file| graph.exported(file.id).to_vec())
        .collect()
}

/// A Convex module file: `x.ts` (Convex has no `index` resolution).
fn module_file(paths: &Paths<'_>, module: &str) -> Option<i64> {
    SOURCE_EXTENSIONS
        .iter()
        .find_map(|ext| paths.get(&format!("{module}.{ext}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_relative_paths() {
        assert_eq!(join_relative("convex/comms/threads", "../../_generated/"), "convex/_generated");
        assert_eq!(join_relative("packages/inbox", "./src/core/index.ts"), "packages/inbox/src/core/index.ts");
        assert_eq!(join_relative("", "scripts/x.ts"), "scripts/x.ts");
    }

    #[test]
    fn chain_base_follows_generated_specifier() {
        let dirs = vec!["convex".to_string()];
        assert_eq!(
            chain_base(&dirs, "convex/comms/threads/drafts.ts", Some("../../_generated/api.js")),
            "convex"
        );
        assert_eq!(
            chain_base(&dirs, "convex/components/analytics/x.ts", Some("./_generated/api")),
            "convex/components/analytics"
        );
        assert_eq!(chain_base(&dirs, "components/a.tsx", Some("@/convex/_generated/api")), "convex");
    }

    #[test]
    fn script_tokens_find_paths_and_function_names() {
        let tokens = script_tokens("bun scripts/dev-signin.ts && bunx convex run comms/doctor:run --prod");
        assert!(tokens.contains(&"scripts/dev-signin.ts".to_string()));
        assert!(tokens.contains(&"comms/doctor:run".to_string()));
        assert!(!tokens.iter().any(|token| token.starts_with("--")));
    }

    #[test]
    fn wildcard_exports() {
        assert!(wildcard_match("packages/ui/src/*.tsx", "packages/ui/src/button.tsx"));
        assert!(!wildcard_match("packages/ui/src/*.tsx", "packages/ui/lib/button.tsx"));
    }
}
