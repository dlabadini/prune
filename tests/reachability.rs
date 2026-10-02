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
