//! Structural rules powered by ast-grep.

use ast_grep_core::matcher::Pattern;
use ast_grep_core::tree_sitter::LanguageExt;
use ast_grep_language::SupportLang;

/// A structural match recorded while indexing a file.
#[derive(Debug, Clone)]
pub struct StructuralHit {
    pub kind: &'static str,
    pub name: String,
    pub line: usize,
    pub detail: String,
}

/// Run barrel and wrapper rules against one source file.
pub fn analyze(source: &str, extension: &str) -> Vec<StructuralHit> {
    let lang = match extension {
        "js" | "mjs" | "cjs" => SupportLang::JavaScript,
        "jsx" | "tsx" => SupportLang::Tsx,
        _ => SupportLang::TypeScript,
    };

    let root = lang.ast_grep(source);
    let mut hits = Vec::new();

    let reexport_patterns = [
        r#"export * from "$M""#,
        r#"export * as $N from "$M""#,
        r#"export { $A } from "$M""#,
        r#"export { $A as $B } from "$M""#,
    ];
    for pattern in reexport_patterns {
        let Ok(pattern) = Pattern::try_new(pattern, lang) else {
            continue;
        };
        for matched in root.root().find_all(pattern) {
            hits.push(StructuralHit {
                kind: "reexport",
                name: matched.text().to_string(),
                line: matched.start_pos().line() + 1,
                detail: matched.text().to_string(),
            });
        }
    }

    let wrapper_patterns = [
        "function $NAME($$$PARAMS) { return $FN($$$ARGS) }",
        "export function $NAME($$$PARAMS) { return $FN($$$ARGS) }",
        "const $NAME = ($$$PARAMS) => $FN($$$ARGS)",
        "const $NAME = ($$$PARAMS) => { return $FN($$$ARGS) }",
        "export const $NAME = ($$$PARAMS) => $FN($$$ARGS)",
    ];
    for pattern in wrapper_patterns {
        let Ok(pattern) = Pattern::try_new(pattern, lang) else {
            continue;
        };
        for matched in root.root().find_all(pattern) {
            // Only a pure pass-through is a wrapper: it forwards exactly its own parameters, in
            // order. A function that adds defaults, constants, or composes calls does real work.
            let env = matched.get_env();
            let params: Vec<String> = env
                .get_multiple_matches("PARAMS")
                .iter()
                .filter(|node| node.is_named())
                .map(|node| param_name(&node.text()))
                .collect();
            let args: Vec<String> = env
                .get_multiple_matches("ARGS")
                .iter()
                .filter(|node| node.is_named())
                .map(|node| node.text().trim().to_string())
                .collect();
            if params != args {
                continue;
            }
            let name = matched
                .get_env()
                .get_match("NAME")
                .map(|node| node.text().to_string())
                .unwrap_or_else(|| "wrapper".to_string());
            let target = matched
                .get_env()
                .get_match("FN")
                .map(|node| node.text().to_string())
                .unwrap_or_default();
            hits.push(StructuralHit {
                kind: "wrapper",
                name,
                line: matched.start_pos().line() + 1,
                detail: target,
            });
        }
    }

    // Overlapping patterns hit the same declaration twice: `function $NAME`
    // matches the declaration inside `export function`, and `const $NAME`
    // matches inside `export const`. Keep one hit per declaration.
    let mut seen = std::collections::HashSet::new();
    hits.retain(|hit| seen.insert((hit.kind, hit.name.clone(), hit.line)));
    hits
}

/// The bound name of a parameter: `d: Date` → `d`, `opts = {}` → `opts`, `...rest` stays `...rest`.
fn param_name(text: &str) -> String {
    let end = text.find([':', '=', '?']).unwrap_or(text.len());
    text[..end].trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_reexports_and_wrappers() {
        let source = r#"
export * from "./a";
export { beta } from "./b";

export function pass(value: string) {
    return trim(value);
}
"#;
        let hits = analyze(source, "ts");
        let reexports: Vec<_> = hits.iter().filter(|hit| hit.kind == "reexport").collect();
        assert!(reexports.len() >= 2, "reexports={reexports:?} all={hits:?}");
        assert!(hits
            .iter()
            .any(|hit| hit.kind == "wrapper" && hit.name == "pass"));
    }

    #[test]
    fn exported_wrappers_are_reported_once() {
        let source = r#"
export function used() { return helper(); }
export function helper() { return 1; }
export const arrow = (x: number) => helper(x);
function local(x: number) { return helper(x); }
"#;
        let hits = analyze(source, "ts");
        for name in ["used", "arrow", "local"] {
            let count = hits
                .iter()
                .filter(|hit| hit.kind == "wrapper" && hit.name == name)
                .count();
            assert_eq!(count, 1, "{name}: {hits:?}");
        }
    }

    #[test]
    fn functions_that_add_work_are_not_wrappers() {
        let source = r#"
const read = (...parts) => fs.readFileSync(path.join(__dirname, ...parts), "utf8");
function clay(color, extra = {}) { return make({ color, roughness: 0.8, ...extra }); }
function wants(req) { return req.accepts(["text/html", "text/markdown"]); }
function swapped(a, b) { return other(b, a); }
function forward(a, b) { return other(a, b); }
const spread = (...args) => other(...args);
"#;
        let wrappers: Vec<_> = analyze(source, "js")
            .into_iter()
            .filter(|hit| hit.kind == "wrapper")
            .map(|hit| hit.name)
            .collect();
        assert_eq!(wrappers, ["forward", "spread"], "{wrappers:?}");
    }
}
