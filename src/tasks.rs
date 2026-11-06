//! Findings as a prioritized task list an agent (or a person) can work top to bottom.
//!
//! Each task is self-contained: where, what was observed, the change, and how to prove it is
//! done (`prune verify <id>`). Findings `prune fix` can apply say so, so an agent runs the fix
//! instead of hand-editing.

use crate::patterns::{Finding, PatternType, Severity};
use crate::score::{finding_outcome, next_line};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Task {
    pub id: String,
    /// 1 is most urgent.
    pub priority: u8,
    pub pattern: PatternType,
    pub severity: Severity,
    pub title: String,
    pub files: Vec<String>,
    /// `file:line` for each location.
    pub locations: Vec<String>,
    pub why: String,
    pub change: String,
    /// Shell command that applies this task safely, when Prune can do it itself.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub autofix: Option<String>,
    /// Exits 0 once the task is done.
    pub done_when: String,
}

impl Task {
    pub fn from_finding(finding: &Finding) -> Self {
        let mut files: Vec<String> = Vec::new();
        for location in &finding.locations {
            if !files.contains(&location.file) {
                files.push(location.file.clone());
            }
        }
        let change = if finding.action_summary.is_empty() {
            next_line(finding)
        } else {
            finding.action_summary.clone()
        };
        Task {
            id: finding.id.clone(),
            priority: priority(finding),
            pattern: finding.pattern_type,
            severity: finding.severity,
            title: finding.title.clone(),
            locations: finding
                .locations
                .iter()
                .map(|location| format!("{}:{}", location.file, location.start_line))
                .collect(),
            files,
            why: first_line(if finding.rationale.trim().is_empty() {
                &finding.description
            } else {
                &finding.rationale
            }),
            change,
            autofix: autofix(finding),
            done_when: format!("prune verify {}", finding.id),
        }
    }
}

/// 1 = high-severity essential (dead code, god files, cycles) ... 4 = low or emerging.
fn priority(finding: &Finding) -> u8 {
    use crate::patterns::Tier;
    let tier = finding.pattern_type.tier();
    let outcome_fail = matches!(finding_outcome(finding), crate::score::Outcome::Fail);
    match (tier, outcome_fail) {
        (Tier::Essential, true) => 1,
        (Tier::Essential, false) | (Tier::Recommended, true) => 2,
        (Tier::Recommended, false) => 3,
        (Tier::Emerging, _) => 4,
    }
}

fn autofix(finding: &Finding) -> Option<String> {
    if finding.pattern_type != PatternType::DeadExports {
        return None;
    }
    let file = finding.symbols.iter().find_map(|symbol| {
        let fixable = symbol.usage == "same_file_only"
            || (symbol.usage == "unreachable" && symbol.name == "*");
        fixable.then_some(symbol.file.as_str())
    })?;
    Some(format!("prune fix --apply --path-prefix {file}"))
}

fn first_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .to_string()
}

/// Tasks for `findings` at or above `min_severity`, most urgent first, at most `limit`.
pub fn build(findings: &[Finding], min_severity: Severity, limit: Option<usize>) -> Vec<Task> {
    let mut tasks: Vec<Task> = findings
        .iter()
        .filter(|finding| finding.severity >= min_severity)
        .map(Task::from_finding)
        .collect();
    tasks.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then(b.severity.cmp(&a.severity))
            .then(a.id.cmp(&b.id))
    });
    if let Some(limit) = limit {
        tasks.truncate(limit);
    }
    tasks
}

/// Markdown checklist an agent can be handed as-is.
pub fn render_markdown(tasks: &[Task], total: usize) -> String {
    let mut out = String::new();
    out.push_str("# Prune tasks\n\n");
    if tasks.is_empty() {
        out.push_str("Nothing to do: no findings at this severity.\n");
        return out;
    }
    out.push_str(&format!(
        "{} of {} findings, most urgent first. Work one task at a time; a task is done when its \
         `done when` command exits 0. Prefer `autofix` when present. Before deleting or renaming \
         an export, ask `prune mcp` tools `safe_to_delete` / `rename_impact` (or run `prune rename` \
         as a dry run).\n\n",
        tasks.len(),
        total
    ));
    for task in tasks {
        out.push_str(&format!(
            "- [ ] **P{} · {}** — {}\n",
            task.priority,
            task.pattern.display_name(),
            task.title
        ));
        out.push_str(&format!("  - id: `{}`\n", task.id));
        if !task.locations.is_empty() {
            out.push_str(&format!("  - where: {}\n", task.locations.join(", ")));
        }
        if !task.why.is_empty() {
            out.push_str(&format!("  - why: {}\n", task.why));
        }
        out.push_str(&format!("  - change: {}\n", task.change));
        if let Some(command) = &task.autofix {
            out.push_str(&format!("  - autofix: `{command}`\n"));
        }
        out.push_str(&format!("  - done when: `{}`\n", task.done_when));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::patterns::{FindingSymbol, Location};

    fn finding(pattern: PatternType, severity: Severity, id: &str) -> Finding {
        let mut finding = Finding::new(
            pattern,
            severity,
            0.9,
            format!("title {id}"),
            "description".into(),
        );
        finding.rationale = "rationale".into();
        finding.locations = vec![Location::new("src/a.ts".into(), 3, 9)];
        finding.suggested_fix = "Do the thing".into();
        finding.id = id.into();
        finding
    }

    #[test]
    fn orders_by_priority_and_filters_severity() {
        let findings = vec![
            finding(PatternType::OverAbstraction, Severity::High, "c"),
            finding(PatternType::GodFile, Severity::High, "a"),
            finding(PatternType::InterfaceSprawl, Severity::Low, "b"),
        ];
        let tasks = build(&findings, Severity::Medium, None);
        let ids: Vec<&str> = tasks.iter().map(|task| task.id.as_str()).collect();
        assert_eq!(ids, ["a", "c"]);
        assert_eq!(tasks[0].priority, 1);
        assert_eq!(tasks[0].done_when, "prune verify a");
    }

    #[test]
    fn dead_same_file_exports_offer_autofix() {
        let mut dead = finding(PatternType::DeadExports, Severity::Medium, "d");
        dead.symbols.push(FindingSymbol {
            name: "helper".into(),
            kind: "function".into(),
            file: "src/a.ts".into(),
            line: 3,
            usage: "same_file_only".into(),
            detail: None,
        });
        let task = Task::from_finding(&dead);
        assert_eq!(
            task.autofix.as_deref(),
            Some("prune fix --apply --path-prefix src/a.ts")
        );
        let markdown = render_markdown(&[task], 1);
        assert!(markdown.contains("- [ ] **P2 · Dead Exports**"));
        assert!(markdown.contains("autofix: `prune fix --apply --path-prefix src/a.ts`"));
    }
}
