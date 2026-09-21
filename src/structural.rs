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

    hits
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
}
