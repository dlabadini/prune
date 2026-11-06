//! Prune - Detect structural and semantic anti-patterns in codebases
//!
//! A local-first developer tool that detects AI-generated code issues
//! and provides a feedback loop for AI-assisted refactoring.

mod cli;
mod tui;

use clap::Parser;
use cli::{Cli, Commands};
use prune::{Analyzer, Config, RefactorPlan, Severity};
use std::collections::HashSet;
use std::io::{self};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

fn main() {
    // Initialize logging
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()),
        ))
        .with(tracing_subscriber::fmt::layer().with_writer(io::stderr))
        .init();

    let cli = Cli::parse();
    let root = cli.project_root();

    // If no command specified, launch TUI
    let result = match cli.command {
        None => tui::run(&root),
        Some(Commands::Scan {
            json,
            watch,
            baseline,
            changed,
            ci,
        }) => cmd_scan(&root, json, watch, ScanFilter { baseline, changed, ci }),
        Some(Commands::Check {
            files,
            all,
            fail_on,
            baseline,
            no_baseline,
            json,
        }) => cmd_check(
            &root,
            CheckOptions {
                files,
                all,
                fail_on,
                baseline,
                no_baseline,
                json,
            },
        ),
        Some(Commands::Tasks {
            json,
            limit,
            min_severity,
            changed,
            baseline,
            output,
        }) => cmd_tasks(
            &root,
            TasksOptions {
                json,
                limit,
                min_severity,
                changed,
                baseline,
                output,
            },
        ),
        Some(Commands::Baseline { output }) => cmd_baseline(&root, &output),
        Some(Commands::Fix {
            apply,
            check,
            only,
            path_prefix,
        }) => cmd_fix(&root, apply, check, only, path_prefix),
        Some(Commands::Rename {
            file,
            from,
            to,
            apply,
            check,
        }) => cmd_rename(&root, &file, &from, &to, apply, check),
        Some(Commands::Plan {
            finding_id,
            copy,
            output,
            rescan,
        }) => cmd_plan(&root, &finding_id, copy, output, rescan),
        Some(Commands::Verify {
            finding_id,
            json,
            quick,
        }) => cmd_verify(&root, &finding_id, json, quick),
        Some(Commands::Init { force }) => cmd_init(&root, force),
        Some(Commands::Config) => cmd_config(&root),
        Some(Commands::Mcp) => cmd_mcp(&root),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn print_findings_summary(findings: &[prune::patterns::Finding]) {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for finding in findings {
        *counts
            .entry(finding.pattern_type.as_str())
            .or_insert(0) += 1;
    }
    println!("Findings by pattern:");
    for (pattern, count) in counts {
        println!("  {pattern}: {count}");
    }
    println!();
}

fn cmd_mcp(root: &std::path::Path) -> io::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    runtime
        .block_on(prune::mcp::serve(root))
        .map_err(|error| io::Error::other(error.to_string()))
}

/// Which findings `scan` reports.
struct ScanFilter {
    baseline: Option<std::path::PathBuf>,
    changed: Option<String>,
    ci: bool,
}

fn under_root(root: &std::path::Path, path: &std::path::Path) -> std::path::PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

/// Drop findings recorded in `baseline`, then keep those touching files changed since `changed`
/// (and the files importing them).
fn filter_findings(
    root: &std::path::Path,
    analyzer: &Analyzer,
    mut findings: Vec<prune::patterns::Finding>,
    baseline: Option<&std::path::Path>,
    changed: Option<&str>,
) -> io::Result<Vec<prune::patterns::Finding>> {
    if let Some(path) = baseline {
        let path = under_root(root, path);
        let baseline = prune::baseline::Baseline::load(&path).map_err(|error| {
            io::Error::other(format!("cannot read baseline {}: {error}", path.display()))
        })?;
        findings = baseline.new_findings(findings);
    }
    if let Some(base) = changed {
        let changed = prune::baseline::changed_files(root, base)?;
        let scope = prune::baseline::with_dependents(analyzer.database(), &changed);
        findings = prune::baseline::touching(findings, &scope);
    }
    Ok(findings)
}

fn parse_severity(value: &str) -> io::Result<Severity> {
    Severity::parse(value).ok_or_else(|| {
        io::Error::other(format!(
            "unknown severity `{value}` (use low, medium, high, or critical)"
        ))
    })
}

struct CheckOptions {
    files: Vec<String>,
    all: bool,
    fail_on: String,
    baseline: std::path::PathBuf,
    no_baseline: bool,
    json: bool,
}

/// Pre-commit gate. Exits 1 when a new finding at or above `fail_on` touches the checked files.
fn cmd_check(root: &std::path::Path, options: CheckOptions) -> io::Result<()> {
    let fail_on = parse_severity(&options.fail_on)?;
    let scope_files: Option<HashSet<String>> = if options.all {
        None
    } else {
        let files: HashSet<String> = if options.files.is_empty() {
            prune::baseline::staged_files(root)?
        } else {
            options
                .files
                .iter()
                .map(|file| {
                    let path = std::path::Path::new(file);
                    let relative = path.strip_prefix(root).unwrap_or(path);
                    relative
                        .to_string_lossy()
                        .trim_start_matches("./")
                        .replace('\\', "/")
                })
                .collect()
        };
        let files: HashSet<String> = files
            .into_iter()
            .filter(|file| {
                std::path::Path::new(file)
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(prune::analyzer::is_supported_extension)
            })
            .collect();
        if files.is_empty() {
            if options.json {
                println!("{}", serde_json::json!({ "passed": true, "checked_files": 0, "findings": [] }));
            } else {
                println!("prune check: no TypeScript/JavaScript files to check.");
            }
            return Ok(());
        }
        Some(files)
    };

    let mut analyzer = Analyzer::new(root).map_err(|e| io::Error::other(e.to_string()))?;
    // A commit hook must never trigger the embedding model download.
    let embeddings = analyzer.config().effective_embeddings_enabled(root)
        && prune::embeddings::embedding_model_ready_for(root);
    analyzer.config_mut().thresholds.embeddings_enabled = Some(embeddings);
    let result = analyzer.scan().map_err(|e| io::Error::other(e.to_string()))?;

    let baseline_path = under_root(root, &options.baseline);
    let use_baseline = !options.no_baseline && baseline_path.exists();
    let mut findings = filter_findings(
        root,
        &analyzer,
        result.findings,
        use_baseline.then_some(baseline_path.as_path()),
        None,
    )?;
    if let Some(files) = &scope_files {
        let mut scope = prune::baseline::with_dependents(analyzer.database(), files);
        if use_baseline {
            // Only new findings fail, so also catch exports this change left unused.
            scope.extend(prune::baseline::with_imports(analyzer.database(), files));
        }
        findings = prune::baseline::touching(findings, &scope);
    }
    let (failing, notes): (Vec<_>, Vec<_>) = findings
        .into_iter()
        .partition(|finding| finding.severity >= fail_on);

    if options.json {
        let output = serde_json::json!({
            "passed": failing.is_empty(),
            "fail_on": fail_on.as_str(),
            "baseline": use_baseline.then(|| options.baseline.display().to_string()),
            "checked_files": scope_files.as_ref().map(|files| files.len()),
            "failing": failing,
            "below_threshold": notes.len(),
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    } else {
        let checked = match &scope_files {
            Some(files) => format!("{} file(s) and their importers", files.len()),
            None => format!("{} files", result.files_scanned),
        };
        if failing.is_empty() {
            println!(
                "prune check: passed ({checked}; {} below {}).",
                notes.len(),
                fail_on.as_str()
            );
        } else {
            println!(
                "prune check: {} new finding(s) at or above {} ({checked}):",
                failing.len(),
                fail_on.as_str()
            );
            println!();
            for finding in &failing {
                println!(
                    "  {} {} — {}",
                    finding.severity.as_str(),
                    finding.pattern_type.display_name(),
                    finding.title
                );
                if let Some(location) = finding.locations.first() {
                    println!("    {}:{}", location.file, location.start_line);
                }
                let next = if finding.action_summary.is_empty() {
                    prune::score::next_line(finding)
                } else {
                    finding.action_summary.clone()
                };
                println!("    Next: {next}");
                println!("    Plan: prune plan {}", finding.id);
            }
            println!();
            if use_baseline {
                println!("Fix them, or accept them with `prune baseline` and commit the baseline.");
            } else {
                println!("Fix them, or record existing debt with `prune baseline` so only new findings fail.");
            }
            println!("Skip once with `git commit --no-verify`.");
        }
    }

    if !failing.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

struct TasksOptions {
    json: bool,
    limit: Option<usize>,
    min_severity: String,
    changed: Option<String>,
    baseline: Option<std::path::PathBuf>,
    output: Option<std::path::PathBuf>,
}

fn cmd_tasks(root: &std::path::Path, options: TasksOptions) -> io::Result<()> {
    let min_severity = parse_severity(&options.min_severity)?;
    let mut analyzer = Analyzer::new(root).map_err(|e| io::Error::other(e.to_string()))?;
    let result = analyzer.scan().map_err(|e| io::Error::other(e.to_string()))?;
    let findings = filter_findings(
        root,
        &analyzer,
        result.findings,
        options.baseline.as_deref(),
        options.changed.as_deref(),
    )?;
    let tasks = prune::tasks::build(&findings, min_severity, options.limit);
    let text = if options.json {
        serde_json::to_string_pretty(&serde_json::json!({
            "total_findings": findings.len(),
            "tasks": tasks,
        }))
        .unwrap()
            + "\n"
    } else {
        prune::tasks::render_markdown(&tasks, findings.len())
    };
    match options.output {
        Some(path) => {
            let path = under_root(root, &path);
            std::fs::write(&path, text)?;
            eprintln!("Wrote {} task(s) to {}", tasks.len(), path.display());
        }
        None => print!("{text}"),
    }
    Ok(())
}

fn cmd_baseline(root: &std::path::Path, output: &std::path::Path) -> io::Result<()> {
    let mut analyzer = Analyzer::new(root).map_err(|e| io::Error::other(e.to_string()))?;
    let result = analyzer.scan().map_err(|e| io::Error::other(e.to_string()))?;
    let path = if output.is_absolute() {
        output.to_path_buf()
    } else {
        root.join(output)
    };
    prune::baseline::Baseline::from_findings(&result.findings).save(&path)?;
    println!(
        "Wrote {} findings to {}. Commit it; `prune scan --baseline --ci` then fails only on new findings.",
        result.findings.len(),
        path.display()
    );
    Ok(())
}

fn cmd_scan(
    root: &std::path::Path,
    json: bool,
    watch: bool,
    filter: ScanFilter,
) -> io::Result<()> {
    let mut analyzer = Analyzer::new(root).map_err(|e| io::Error::other(e.to_string()))?;

    let mut result = analyzer
        .scan()
        .map_err(|e| io::Error::other(e.to_string()))?;
    let total_findings = result.findings.len();
    result.findings = filter_findings(
        root,
        &analyzer,
        std::mem::take(&mut result.findings),
        filter.baseline.as_deref(),
        filter.changed.as_deref(),
    )?;
    let filtered = result.findings.len() != total_findings;

    if json {
        let output = serde_json::json!({
            "version": result.score.version,
            "score": result.score,
            "files_scanned": result.files_scanned,
            "files_changed": result.files_changed,
            "total_lines": result.total_lines,
            "symbols_found": result.symbols_found,
            "scan_duration_ms": result.scan_duration_ms,
            "total_findings": total_findings,
            "findings": result.findings,
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    } else {
        println!(
            "Prune  {}    {}",
            result.score.summary(),
            result.score.version
        );
        println!(
            "Essential {:.1}/{:.0}   Recommended {:.1}/{:.0}   Bonus {:.1}",
            result.score.essential_earned,
            result.score.essential_available,
            result.score.recommended_earned,
            result.score.recommended_available,
            result.score.bonus
        );
        println!(
            "Scanned {} files ({} lines) in {}ms",
            result.files_scanned, result.total_lines, result.scan_duration_ms
        );
        println!();

        let config = Config::find_and_load(root).unwrap_or_default();
        let suggestions = prune::suggest::from_findings(&result.findings, &config);
        prune::suggest::print_suggestions(&suggestions);

        if filtered {
            println!(
                "Showing {} of {} findings (baseline/changed filter).",
                result.findings.len(),
                total_findings
            );
        }
        if result.findings.is_empty() {
            println!("No issues found.");
        } else {
            print_findings_summary(&result.findings);
            let show = result.findings.len().min(25);
            for finding in result.findings.iter().take(show) {
                println!(
                    "{} {}  {} — {}",
                    finding.pattern_type.tier().as_str(),
                    prune::score::finding_outcome(finding).as_str(),
                    finding.pattern_type.display_name(),
                    finding.title
                );
                println!("    {}", finding.id);
                for loc in &finding.locations {
                    println!("    {}:{}", loc.file, loc.start_line);
                }
                if !finding.action_summary.is_empty() {
                    println!("    Prune: {}", finding.action_summary);
                }
                println!("    Next: {}", prune::score::next_line(finding));
                println!();
            }
            if result.findings.len() > show {
                println!(
                    "... and {} more findings. Use `prune scan --json` for the full list.",
                    result.findings.len() - show
                );
            }
        }
    }

    if watch {
        run_headless_watch(&mut analyzer)?;
    }

    if filter.ci && !result.findings.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

fn cmd_fix(
    root: &std::path::Path,
    apply: bool,
    check: Option<String>,
    only: Vec<String>,
    path_prefix: Option<String>,
) -> io::Result<()> {
    let mut analyzer = Analyzer::new(root).map_err(|e| io::Error::other(e.to_string()))?;
    let result = analyzer.scan().map_err(|e| io::Error::other(e.to_string()))?;
    let options = prune::fix::FixOptions {
        apply,
        check: check.or_else(|| analyzer.config().fix.check.clone()),
        kinds: prune::fix::FixKind::parse_list(&only),
        path_prefix,
    };
    let report = prune::fix::run(root, analyzer.database(), &result.findings, &options)?;
    print!("{}", report.render(apply));
    Ok(())
}

fn cmd_rename(
    root: &std::path::Path,
    file: &str,
    from: &str,
    to: &str,
    apply: bool,
    check: Option<String>,
) -> io::Result<()> {
    let mut analyzer = Analyzer::new(root).map_err(|e| io::Error::other(e.to_string()))?;
    analyzer.scan().map_err(|e| io::Error::other(e.to_string()))?;
    let check = check.or_else(|| analyzer.config().fix.check.clone());
    let plan = prune::fix::plan_rename(analyzer.database(), root, file, from, to)?;
    print!("{}", plan.render());
    if apply {
        let outcome = prune::fix::apply_rename(root, &plan, check.as_deref())?;
        println!("{outcome}");
    } else {
        println!("Dry run. Re-run with --apply to write.");
    }
    Ok(())
}

fn run_headless_watch(analyzer: &mut Analyzer) -> io::Result<()> {
    use prune::watcher::{FileWatcher, WatchEvent};
    use std::time::Duration;

    let root = analyzer.root().to_path_buf();
    let mut watcher = FileWatcher::new(&root, 400).map_err(|e| io::Error::other(e.to_string()))?;
    watcher
        .start()
        .map_err(|e| io::Error::other(e.to_string()))?;

    println!("Watching {} — Ctrl+C to stop", root.display());
    let mut last_score: Option<f32> = None;
    let mut last_findings = 0usize;

    loop {
        if let Some(events) = watcher.recv_timeout(Duration::from_millis(400)) {
            for event in events {
                match event {
                    WatchEvent::Modified(path) | WatchEvent::Created(path) => {
                        if let Err(error) = analyzer.reindex_file(&path) {
                            eprintln!("Reindex error: {error}");
                        }
                    }
                    WatchEvent::Deleted(path) => {
                        let _ = analyzer.reindex_file(&path);
                    }
                    WatchEvent::Error(message) => eprintln!("Watch error: {message}"),
                }
            }
            let result = analyzer
                .analyze()
                .map_err(|e| io::Error::other(e.to_string()))?;
            let delta = last_score
                .map(|previous| result.score.total - previous)
                .unwrap_or(0.0);
            let finding_delta = result.findings.len() as i64 - last_findings as i64;
            println!(
                "Score {:.0} ({:+.0})  ·  {} findings ({:+})  ·  {}ms",
                result.score.total,
                delta,
                result.findings.len(),
                finding_delta,
                result.scan_duration_ms
            );
            last_score = Some(result.score.total);
            last_findings = result.findings.len();
        }
    }
}

fn cmd_verify(
    root: &std::path::Path,
    finding_id: &str,
    json: bool,
    quick: bool,
) -> io::Result<()> {
    let mut analyzer = Analyzer::new(root).map_err(|e| io::Error::other(e.to_string()))?;
    let result = prune::verify_finding(&mut analyzer, finding_id, quick)
        .map_err(|e| io::Error::other(e.to_string()))?;

    if json {
        println!("{}", serde_json::to_string_pretty(&result).unwrap());
    } else {
        println!(
            "{}  {}",
            if result.resolved { "✓" } else { "✗" },
            result.message
        );
        if !result.resolved && !result.related_still_present.is_empty() {
            for title in &result.related_still_present {
                println!("  · {title}");
            }
        }
        println!("Rescan: prune verify {finding_id}");
    }
    if !result.resolved {
        std::process::exit(1);
    }
    Ok(())
}

fn cmd_plan(
    root: &std::path::Path,
    finding_id: &str,
    copy: bool,
    output: Option<std::path::PathBuf>,
    rescan: bool,
) -> io::Result<()> {
    let mut analyzer = Analyzer::new(root).map_err(|e| io::Error::other(e.to_string()))?;

    let result = if rescan {
        analyzer
            .scan()
            .map_err(|e| io::Error::other(e.to_string()))?
    } else {
        analyzer
            .analyze()
            .map_err(|e| io::Error::other(e.to_string()))?
    };

    // Find the requested finding
    let finding = result
        .findings
        .iter()
        .find(|f| f.id == finding_id || f.id.starts_with(finding_id))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("Finding '{}' not found", finding_id),
            )
        })?;

    let plan = RefactorPlan::generate(finding);

    if copy {
        match arboard::Clipboard::new() {
            Ok(mut clipboard) => {
                clipboard
                    .set_text(plan.as_markdown())
                    .map_err(|e| io::Error::other(e.to_string()))?;
                println!("Plan copied to clipboard");
            }
            Err(e) => {
                return Err(io::Error::other(format!("Clipboard error: {}", e)));
            }
        }
    } else if let Some(path) = output {
        plan.write_to_file(&path)?;
        println!("Plan saved to {}", path.display());
    } else {
        println!("{}", plan.as_markdown());
    }

    Ok(())
}

fn cmd_init(root: &std::path::Path, force: bool) -> io::Result<()> {
    let config_path = Config::config_path(root);

    if config_path.exists() && !force {
        println!("Config already exists at {}", config_path.display());
    } else {
        let config = Config::default();
        config
            .save(&config_path)
            .map_err(|e| io::Error::other(e.to_string()))?;
        println!("Created {}", config_path.display());
    }

    prune::workspace::ensure_data_dir(root)?;
    if prune::workspace::ensure_gitignore(root)? {
        println!("Added .prune/ to .gitignore");
    }

    let config = Config::find_and_load(root).unwrap_or_default();
    let suggestions = prune::suggest::from_filesystem(root, &config);
    prune::suggest::print_suggestions(&suggestions);

    println!();
    println!("Commit .prunerc. Leave .prune/ untracked — it is the local index.");
    println!("Then run `prune` or `prune scan` in this repo.");

    Ok(())
}

fn cmd_config(root: &std::path::Path) -> io::Result<()> {
    let config = Config::find_and_load(root).map_err(|e| io::Error::other(e.to_string()))?;

    let config_path = Config::config_path(root);
    let config_exists = config_path.exists();

    println!("Prune Configuration");
    println!("===================");
    println!();
    println!(
        "Config file: {}",
        if config_exists {
            config_path.display().to_string()
        } else {
            "(using defaults)".to_string()
        }
    );
    println!();
    println!("Strictness: {}", config.strictness.as_str());
    println!();
    println!("Effective thresholds:");
    println!(
        "  Interface sprawl similarity: {:.0}%",
        config.effective_interface_sprawl_similarity() * 100.0
    );
    println!(
        "  Interface sprawl min cluster: {}",
        config.effective_interface_sprawl_min_cluster()
    );
    println!(
        "  Interface sprawl min bundle fields: {}",
        config.effective_interface_sprawl_min_bundle_fields()
    );
    println!(
        "  Interface sprawl min overlap ratio: {:.0}%",
        config.effective_interface_sprawl_min_overlap_ratio() * 100.0
    );
    println!(
        "  Redundant logic similarity: {:.0}%",
        config.effective_redundant_logic_similarity() * 100.0
    );
    println!(
        "  God file max exports: {}",
        config.effective_god_file_max_exports()
    );
    println!(
        "  God file max lines: {}",
        config.effective_god_file_max_lines()
    );
    println!(
        "  Barrel bloat threshold: {}",
        config.effective_barrel_bloat_threshold()
    );
    println!();
    println!("Exclude patterns: {:?}", config.exclude);
    println!("Disabled patterns: {:?}", config.disabled_patterns);

    Ok(())
}
