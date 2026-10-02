//! End-to-end reachability on a small monorepo fixture (Next app, Convex, workspace package).

use prune::patterns::{Finding, PatternType};
use prune::Analyzer;
use std::collections::HashMap;
use std::path::Path;

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn scan_fixture(name: &str) -> (tempfile::TempDir, Vec<Finding>) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    let dir = tempfile::tempdir().unwrap();
    copy_dir(&source, dir.path());
    let mut analyzer = Analyzer::new(dir.path()).unwrap();
    let result = analyzer.scan().unwrap();
    (dir, result.findings)
}

/// `file::name` → usage, over every dead-export finding.
fn usages(findings: &[Finding]) -> HashMap<String, String> {
    findings
        .iter()
        .filter(|finding| finding.pattern_type == PatternType::DeadExports)
        .flat_map(|finding| finding.symbols.iter())
        .map(|symbol| (format!("{}::{}", symbol.file, symbol.name), symbol.usage.clone()))
        .collect()
}

#[test]
fn classifies_exports_by_reachability() {
    let (_dir, findings) = scan_fixture("reachability");
    let usage = usages(&findings);
    let expect = |key: &str, wanted: Option<&str>| {
        assert_eq!(
            usage.get(key).map(String::as_str),
            wanted,
            "{key}: all = {usage:#?}"
        );
    };

    // Workspace package behind a barrel (`export *`, aliased re-export, type re-export).
    expect("packages/lib/src/a.ts::usedA", None);
    expect("packages/lib/src/a.ts::sameFileA", Some("same_file_only"));
    expect("packages/lib/src/a.ts::unusedA", Some("unused"));
    expect("packages/lib/src/a.ts::testOnlyA", Some("test_only"));
    expect("packages/lib/src/b.ts::b", None);
    expect("packages/lib/src/b.ts::unusedB", Some("unused"));
    expect("packages/lib/src/types.ts::Shape", None);
    expect("packages/lib/src/types.ts::UnusedShape", Some("unused"));

    // Namespace import member, and a package.json script entry.
    expect("app/format.ts::money", None);
    expect("app/format.ts::percent", None);

    // Next.js route exports are framework roots.
    expect("app/dashboard/page.tsx::metadata", None);
    expect("app/dashboard/page.tsx::default", None);

    // Convex: api/internal chains, function-name strings, CLI scripts.
    expect("convex/messages.ts::send", None);
    expect("convex/messages.ts::deliver", None);
    expect("convex/messages.ts::viaString", None);
    expect("convex/messages.ts::runFromCli", None);
    expect("convex/messages.ts::neverCalled", Some("unused"));
    expect("convex/messages.ts::listPublic", Some("unused"));
    expect("convex/lib/helpers.ts::helper", None);
    expect("convex/lib/helpers.ts::unusedHelper", Some("unused"));
    expect("convex/schema.ts::default", None);

    // Files nothing reaches, including one only an unreachable file imports.
    expect("orphans/orphan.ts::*", Some("unreachable"));
    expect("orphans/leaf.ts::*", Some("unreachable"));
    expect("orphans/leaf.ts::onlyFromOrphan", Some("unreachable"));

    // Configured entry keeps its imports alive.
    expect("cycle/x.ts::fromX", None);
    expect("cycle/t2.ts::T2", None);
}

#[test]
fn dead_export_ids_are_stable_across_scans() {
    let (dir, first) = scan_fixture("reachability");
    let mut analyzer = Analyzer::new(dir.path()).unwrap();
    let second = analyzer.scan().unwrap().findings;
    let ids = |findings: &[Finding]| {
        let mut ids: Vec<_> = findings.iter().map(|f| f.id.clone()).collect();
        ids.sort();
        ids
    };
    assert_eq!(ids(&first), ids(&second));
}

#[test]
fn reports_value_and_type_cycles_separately() {
    let (_dir, findings) = scan_fixture("reachability");
    let cycles: Vec<&Finding> = findings
        .iter()
        .filter(|finding| finding.pattern_type == PatternType::ImportCycle)
        .collect();
    let value = cycles
        .iter()
        .find(|finding| finding.locations.iter().any(|l| l.file == "cycle/x.ts"))
        .expect("value cycle x <-> y");
    assert!(value.related_concepts.iter().any(|c| c == "cycle:value"), "{:?}", value.related_concepts);
    let types = cycles
        .iter()
        .find(|finding| finding.locations.iter().any(|l| l.file == "cycle/t1.ts"))
        .expect("type-only cycle t1 <-> t2");
    assert!(types.related_concepts.iter().any(|c| c == "cycle:type"));
    assert!(types.severity < value.severity);
}

#[test]
fn shapes_respect_fenced_boundaries_and_count_validators() {
    let (_dir, findings) = scan_fixture("shapes");
    let shape_findings: Vec<&Finding> = findings
        .iter()
        .filter(|finding| {
            matches!(
                finding.pattern_type,
                PatternType::InterfaceSprawl | PatternType::Lexicon
            )
        })
        .collect();
    let files = |finding: &Finding| -> Vec<String> {
        finding.locations.iter().map(|location| location.file.clone()).collect()
    };
    // The validator joins the hand-written duplicates inside the app boundary.
    let app = shape_findings
        .iter()
        .find(|finding| files(finding).iter().any(|file| file == "convex/threads.ts"))
        .expect("validator duplicates the interfaces");
    assert!(files(app).iter().any(|file| file == "components/thread-row.tsx"));
    assert!(app.title.starts_with("[app]"), "{}", app.title);
    // Nothing crosses the inbox fence.
    for finding in &shape_findings {
        let touches_inbox = files(finding).iter().any(|file| file.starts_with("packages/inbox/"));
        let touches_app = files(finding).iter().any(|file| !file.starts_with("packages/inbox/"));
        assert!(!(touches_inbox && touches_app), "crossed the fence: {}", finding.title);
    }
    // Glossary: `draft` names a held reply and a bag of reply inputs.
    assert!(
        shape_findings
            .iter()
            .any(|finding| finding.title.contains("`draft` names 2 different concepts")),
        "{:#?}",
        shape_findings.iter().map(|f| f.title.clone()).collect::<Vec<_>>()
    );
    // Acronym names are not "resource smash" by default.
    assert!(!shape_findings.iter().any(|finding| finding.title.contains("ThreadUIAction")));
}

#[test]
fn fix_unexports_and_reverts_on_failed_check() {
    let (dir, findings) = scan_fixture("reachability");
    let mut analyzer = Analyzer::new(dir.path()).unwrap();
    analyzer.scan().unwrap();
    let a_path = dir.path().join("packages/lib/src/a.ts");
    let before = std::fs::read_to_string(&a_path).unwrap();

    // A failing check reverts the edit.
    let options = prune::fix::FixOptions {
        apply: true,
        check: Some("false".into()),
        kinds: vec![prune::fix::FixKind::Unexport],
        path_prefix: Some("packages/lib/".into()),
    };
    let report = prune::fix::run(dir.path(), analyzer.database(), &findings, &options).unwrap();
    assert_eq!(report.reverted.len(), 1);
    assert_eq!(std::fs::read_to_string(&a_path).unwrap(), before);

    // A passing check keeps it.
    let options = prune::fix::FixOptions {
        check: Some("true".into()),
        ..options
    };
    let report = prune::fix::run(dir.path(), analyzer.database(), &findings, &options).unwrap();
    assert_eq!(report.applied, vec!["packages/lib/src/a.ts".to_string()]);
    let after = std::fs::read_to_string(&a_path).unwrap();
    assert!(after.contains("\nfunction sameFileA()"), "{after}");
    assert!(after.contains("export function unusedA()"));
}

#[test]
fn rename_follows_barrels_and_importers() {
    let (dir, _) = scan_fixture("reachability");
    let mut analyzer = Analyzer::new(dir.path()).unwrap();
    analyzer.scan().unwrap();
    let plan = prune::fix::plan_rename(
        analyzer.database(),
        dir.path(),
        "packages/lib/src/b.ts",
        "b",
        "bValue",
    )
    .unwrap();
    assert!(plan.refused.is_none());
    let files = plan.files();
    assert!(files.contains(&"packages/lib/src/b.ts".to_string()), "{files:?}");
    assert!(files.contains(&"packages/lib/src/index.ts".to_string()), "{files:?}");
    prune::fix::apply_rename(dir.path(), &plan, Some("true")).unwrap();
    let barrel = std::fs::read_to_string(dir.path().join("packages/lib/src/index.ts")).unwrap();
    assert!(barrel.contains("bValue as renamed"), "{barrel}");
    // The importer uses the barrel's alias, which does not change.
    let page = std::fs::read_to_string(dir.path().join("app/dashboard/page.tsx")).unwrap();
    assert!(page.contains("renamed()"));

    // Convex functions are refused.
    let plan = prune::fix::plan_rename(
        analyzer.database(),
        dir.path(),
        "convex/messages.ts",
        "send",
        "sendMessage",
    )
    .unwrap();
    assert!(plan.refused.is_some());
}

#[test]
fn baseline_reports_only_new_findings() {
    let (dir, findings) = scan_fixture("reachability");
    let baseline = prune::baseline::Baseline::from_findings(&findings);
    std::fs::write(
        dir.path().join("orphans/another.ts"),
        "export function stray() { return 1; }\n",
    )
    .unwrap();
    let mut analyzer = Analyzer::new(dir.path()).unwrap();
    let fresh = baseline.new_findings(analyzer.scan().unwrap().findings);
    assert_eq!(fresh.len(), 1, "{:#?}", fresh.iter().map(|f| &f.title).collect::<Vec<_>>());
    assert!(fresh[0].title.contains("another.ts"));
}

#[test]
fn changed_scope_includes_dependents() {
    let (dir, findings) = scan_fixture("reachability");
    let mut analyzer = Analyzer::new(dir.path()).unwrap();
    analyzer.scan().unwrap();
    let changed: std::collections::HashSet<String> =
        ["packages/lib/src/a.ts".to_string()].into_iter().collect();
    let scope = prune::baseline::with_dependents(analyzer.database(), &changed);
    assert!(scope.contains("packages/lib/src/index.ts"));
    assert!(scope.contains("app/dashboard/page.tsx"));
    assert!(!scope.contains("convex/messages.ts"));
    let scoped = prune::baseline::touching(findings, &scope);
    assert!(scoped
        .iter()
        .all(|finding| finding.locations.iter().any(|l| scope.contains(&l.file))));
    assert!(!scoped.is_empty());

    // git-backed: an untracked file shows up as changed.
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap()
    };
    if git(&["init", "-q"]).status.success() {
        git(&["add", "-A"]);
        git(&["-c", "user.email=t@t", "-c", "user.name=t", "commit", "-qm", "init"]);
        std::fs::write(dir.path().join("app/new.ts"), "export const x = 1;\n").unwrap();
        let changed = prune::baseline::changed_files(dir.path(), "HEAD").unwrap();
        assert!(changed.contains("app/new.ts"), "{changed:?}");
    }
}

#[test]
fn agent_queries_answer_who_uses_and_safe_to_delete() {
    let (dir, _) = scan_fixture("reachability");
    let mut analyzer = Analyzer::new(dir.path()).unwrap();
    analyzer.scan().unwrap();
    let report = prune::graph::usage::UsageReport::load(analyzer.database()).unwrap();

    let used = report.symbol_usage("packages/lib/src/a.ts", "usedA").unwrap();
    assert!(used.users.iter().any(|user| user.file == "app/dashboard/page.tsx"));
    assert!(used.reexported_by.contains(&"packages/lib/src/index.ts".to_string()));
    let verdict = report.delete_verdict("packages/lib/src/a.ts", "usedA").unwrap();
    assert!(!verdict.safe_to_delete);

    let verdict = report.delete_verdict("packages/lib/src/a.ts", "unusedA").unwrap();
    assert!(verdict.safe_to_delete, "{:?}", verdict.reasons);

    let verdict = report.delete_verdict("packages/lib/src/a.ts", "testOnlyA").unwrap();
    assert!(verdict.safe_to_delete);
    assert!(verdict.reasons.iter().any(|reason| reason.contains("test file")));

    let verdict = report.delete_verdict("convex/messages.ts", "deliver").unwrap();
    assert!(!verdict.safe_to_delete);
    let verdict = report.delete_verdict("app/dashboard/page.tsx", "metadata").unwrap();
    assert!(!verdict.safe_to_delete, "framework root");
}

#[test]
fn fix_deletes_import_connected_unreachable_files_together() {
    let (dir, findings) = scan_fixture("reachability");
    let mut analyzer = Analyzer::new(dir.path()).unwrap();
    analyzer.scan().unwrap();
    // The check fails while orphan.ts still imports a deleted leaf.ts.
    let check = "! test -e orphans/orphan.ts || test -e orphans/leaf.ts";
    let options = prune::fix::FixOptions {
        apply: true,
        check: Some(check.into()),
        kinds: vec![prune::fix::FixKind::DeleteFile],
        path_prefix: None,
    };
    let report = prune::fix::run(dir.path(), analyzer.database(), &findings, &options).unwrap();
    assert!(report.reverted.is_empty(), "{:?}", report.reverted);
    assert!(!dir.path().join("orphans/leaf.ts").exists());
    assert!(!dir.path().join("orphans/orphan.ts").exists());

    // Applying without a check is refused.
    let options = prune::fix::FixOptions {
        check: None,
        ..options
    };
    assert!(prune::fix::run(dir.path(), analyzer.database(), &findings, &options).is_err());
}
