//! Structured compose/refactor plans — the machine-readable pruning instructions.

use crate::patterns::interface_sprawl::type_shape::{FieldBinding, infer_bundle_name};
use serde::{Deserialize, Serialize};

/// Machine-readable pruning action attached to a finding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruneAction {
    pub kind: PruneActionKind,
    pub target_module: String,
    pub canonical_name: String,
    pub bundles: Vec<BundleSpec>,
    pub steps: Vec<ComposeStep>,
    pub members: Vec<MemberCompose>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PruneActionKind {
    ComposeBundles,
    RenameLexicon,
    ConsolidateDuplicate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleSpec {
    pub name: String,
    pub fields: Vec<String>,
    pub parent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeStep {
    pub order: usize,
    pub title: String,
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemberCompose {
    pub type_name: String,
    pub file: String,
    pub line: usize,
    pub extends: Vec<String>,
    pub unique_fields: Vec<String>,
}

pub struct SprawlComposeInput {
    pub concept: String,
    pub shared_fields: Vec<FieldBinding>,
    pub child_bundles: Vec<ChildBundleInput>,
    pub members: Vec<MemberInput>,
}

pub struct ChildBundleInput {
    pub name: String,
    pub fields: Vec<FieldBinding>,
}

pub struct MemberInput {
    pub type_name: String,
    pub file: String,
    pub line: usize,
    pub bindings: Vec<FieldBinding>,
}

pub fn compose_from_sprawl(input: &SprawlComposeInput) -> PruneAction {
    let target_module = infer_target_module(&input.members);
    let root_name = if input.concept.ends_with("Facts") {
        input.concept.clone()
    } else {
        infer_bundle_name(&input.shared_fields)
    };

    let bundles = build_bundle_specs(&root_name, &input.shared_fields, &input.child_bundles);
    let steps = build_compose_steps(&target_module, &bundles);
    let members: Vec<MemberCompose> = input
        .members
        .iter()
        .map(|member| member_compose(member, &input.shared_fields, &bundles))
        .collect();

    PruneAction {
        kind: if input.child_bundles.len() > 1 {
            PruneActionKind::ComposeBundles
        } else if members.len() > 1 && members.iter().all(|m| m.unique_fields.is_empty()) {
            PruneActionKind::ConsolidateDuplicate
        } else {
            PruneActionKind::ComposeBundles
        },
        target_module,
        canonical_name: root_name,
        bundles,
        steps,
        members,
    }
}

pub fn compose_from_lexicon(
    canonical: &str,
    homonyms: &[(&str, &str, usize)],
    field_signature: &str,
) -> PruneAction {
    let target_module = infer_target_module_from_paths(
        &homonyms
            .iter()
            .map(|(_, file, _)| file.to_string())
            .collect::<Vec<_>>(),
    );
    let steps = vec![
        ComposeStep {
            order: 1,
            title: format!("Pick canonical name `{}`", canonical),
            code: format!(
                "// Same field shape: {}\n// Rename divergent types to share one concept namespace.",
                field_signature
            ),
        },
        ComposeStep {
            order: 2,
            title: "Rename homonyms".to_string(),
            code: homonyms
                .iter()
                .map(|(name, file, line)| {
                    format!("// {} at {}:{} → align with {}", name, file, line, canonical)
                })
                .collect::<Vec<_>>()
                .join("\n"),
        },
    ];
    let members = homonyms
        .iter()
        .map(|(name, file, line)| MemberCompose {
            type_name: name.to_string(),
            file: file.to_string(),
            line: *line,
            extends: Vec::new(),
            unique_fields: Vec::new(),
        })
        .collect();

    PruneAction {
        kind: PruneActionKind::RenameLexicon,
        target_module,
        canonical_name: canonical.to_string(),
        bundles: Vec::new(),
        steps,
        members,
    }
}

fn build_bundle_specs(
    root_name: &str,
    shared_fields: &[FieldBinding],
    child_bundles: &[ChildBundleInput],
) -> Vec<BundleSpec> {
    let mut specs = Vec::new();
    if child_bundles.len() > 1 {
        for child in child_bundles {
            specs.push(BundleSpec {
                name: child.name.clone(),
                fields: child.fields.iter().map(|f| f.name.clone()).collect(),
                parent: None,
            });
        }
        specs.push(BundleSpec {
            name: root_name.to_string(),
            fields: shared_fields.iter().map(|f| f.name.clone()).collect(),
            parent: None,
        });
    } else {
        specs.push(BundleSpec {
            name: root_name.to_string(),
            fields: shared_fields.iter().map(|f| f.name.clone()).collect(),
            parent: None,
        });
    }
    specs
}

fn build_compose_steps(target_module: &str, bundles: &[BundleSpec]) -> Vec<ComposeStep> {
    let mut steps = Vec::new();
    for (index, bundle) in bundles.iter().enumerate() {
        let fields = bundle
            .fields
            .iter()
            .map(|name| format!("  {}: unknown;", name))
            .collect::<Vec<_>>()
            .join("\n");
        steps.push(ComposeStep {
            order: index + 1,
            title: format!("Extract `{}`", bundle.name),
            code: format!(
                "// {}\nexport interface {} {{\n{}\n}}",
                target_module, bundle.name, fields
            ),
        });
    }
    steps.push(ComposeStep {
        order: bundles.len() + 1,
        title: "Compose consumers with extends".to_string(),
        code: "// Replace inline fields with `extends BundleName` per member.".to_string(),
    });
    steps
}

fn member_compose(
    member: &MemberInput,
    shared: &[FieldBinding],
    bundles: &[BundleSpec],
) -> MemberCompose {
    let shared_names: std::collections::HashSet<_> =
        shared.iter().map(|f| f.name.as_str()).collect();
    let unique_fields = member
        .bindings
        .iter()
        .filter(|b| !shared_names.contains(b.name.as_str()))
        .map(|b| b.name.clone())
        .collect();
    let extends = if bundles.len() == 1 {
        vec![bundles[0].name.clone()]
    } else if bundles.len() > 1 {
        vec![bundles.last().map(|b| b.name.clone()).unwrap_or_default()]
    } else {
        Vec::new()
    };
    MemberCompose {
        type_name: member.type_name.clone(),
        file: member.file.clone(),
        line: member.line,
        extends,
        unique_fields,
    }
}

fn infer_target_module(members: &[MemberInput]) -> String {
    infer_target_module_from_paths(&members.iter().map(|m| m.file.clone()).collect::<Vec<_>>())
}

fn infer_target_module_from_paths(files: &[String]) -> String {
    if files.is_empty() {
        return "src/types/domain.ts".to_string();
    }
    let mut domain = 0;
    let mut packages = 0;
    let mut convex = 0;
    for file in files {
        if file.contains("/packages/") || file.starts_with("packages/") {
            packages += 1;
        } else if file.starts_with("convex/") {
            convex += 1;
        } else if file.contains("/domain/") {
            domain += 1;
        }
    }
    if packages > 0 {
        "packages/domain/src/types/facts.ts".to_string()
    } else if convex > files.len() / 2 {
        "convex/types/facts.ts".to_string()
    } else if domain > 0 {
        "packages/domain/src/types/facts.ts".to_string()
    } else {
        "src/types/domain.ts".to_string()
    }
}

pub fn format_action_summary(action: &PruneAction) -> String {
    match action.kind {
        PruneActionKind::ComposeBundles => {
            let bundle_names = action
                .bundles
                .iter()
                .map(|b| b.name.as_str())
                .collect::<Vec<_>>()
                .join(" + ");
            format!(
                "Extract {} → compose {} member(s) via `extends`",
                bundle_names,
                action.members.len()
            )
        }
        PruneActionKind::RenameLexicon => format!(
            "Align {} homonym(s) under canonical `{}`",
            action.members.len(),
            action.canonical_name
        ),
        PruneActionKind::ConsolidateDuplicate => format!(
            "Consolidate into `{}` in {}",
            action.canonical_name,
            action.target_module
        ),
    }
}

pub fn format_action_markdown(action: &PruneAction) -> String {
    let mut md = String::new();
    md.push_str("## Prune compose plan\n\n");
    md.push_str(&format!("**Target:** `{}`\n\n", action.target_module));
    md.push_str(&format!("**Canonical:** `{}`\n\n", action.canonical_name));
    if !action.bundles.is_empty() {
        md.push_str("### Bundles\n\n");
        for bundle in &action.bundles {
            md.push_str(&format!(
                "- `{}` — {}\n",
                bundle.name,
                bundle.fields.join(", ")
            ));
        }
        md.push('\n');
    }
    md.push_str("### Steps\n\n");
    for step in &action.steps {
        md.push_str(&format!("{}. {}\n\n", step.order, step.title));
        md.push_str("```typescript\n");
        md.push_str(&step.code);
        md.push_str("\n```\n\n");
    }
    if !action.members.is_empty() {
        md.push_str("### Members\n\n");
        for member in &action.members {
            let extends = if member.extends.is_empty() {
                String::new()
            } else {
                format!(" extends {}", member.extends.join(" + "))
            };
            let extras = if member.unique_fields.is_empty() {
                String::new()
            } else {
                format!(" (+ {})", member.unique_fields.join(", "))
            };
            md.push_str(&format!(
                "- `{}` at {}:{}{}\n",
                member.type_name,
                member.file,
                member.line,
                format!("{}{}", extends, extras)
            ));
        }
    }
    md
}
