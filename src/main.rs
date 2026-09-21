//! Prune - Detect structural and semantic anti-patterns in codebases
//!
//! A local-first developer tool that detects AI-generated code issues
//! and provides a feedback loop for AI-assisted refactoring.

mod cli;
mod tui;

use clap::Parser;
use cli::{Cli, Commands, LicenseAction};
use prune::{Analyzer, Config, LicenseManager, LicenseStatus, LicenseTier, RefactorPlan};
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
        Some(Commands::Scan { json, watch }) => cmd_scan(&root, json, watch),
        Some(Commands::Plan {
            finding_id,
            copy,
            output,
        }) => cmd_plan(&root, &finding_id, copy, output),
        Some(Commands::Init { force }) => cmd_init(&root, force),
        Some(Commands::Config) => cmd_config(&root),
        Some(Commands::License { action }) => cmd_license(action),
    };

    if let Err(e) = result {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn cmd_scan(root: &std::path::Path, json: bool, watch: bool) -> io::Result<()> {
    let mut analyzer = Analyzer::new(root).map_err(|e| io::Error::other(e.to_string()))?;

    let result = analyzer
        .scan()
        .map_err(|e| io::Error::other(e.to_string()))?;

    if json {
        let output = serde_json::json!({
            "files_scanned": result.files_scanned,
            "files_changed": result.files_changed,
            "total_lines": result.total_lines,
            "symbols_found": result.symbols_found,
            "scan_duration_ms": result.scan_duration_ms,
            "findings": result.findings,
        });
        println!("{}", serde_json::to_string_pretty(&output).unwrap());
    } else {
        println!("Prune Scan Results");
        println!("==================");
        println!();
        println!(
            "Scanned {} files ({} lines) in {}ms",
            result.files_scanned, result.total_lines, result.scan_duration_ms
        );
        println!("Found {} symbols", result.symbols_found);
        println!();

        if result.findings.is_empty() {
            println!("No issues found! Your codebase looks clean.");
        } else {
            println!("Found {} issues:", result.findings.len());
            println!();

            for finding in &result.findings {
                let severity_indicator = match finding.severity {
                    prune::Severity::Critical => "[!!!!]",
                    prune::Severity::High => "[!! ]",
                    prune::Severity::Medium => "[!  ]",
                    prune::Severity::Low => "[.  ]",
                };

                println!(
                    "{} {} - {}",
                    severity_indicator,
                    finding.pattern_type.display_name(),
                    finding.title
                );
                println!("    ID: {}", finding.id);

                for loc in &finding.locations {
                    println!(
                        "    {} (lines {}-{})",
                        loc.file, loc.start_line, loc.end_line
                    );
                }
                println!();
            }
        }
    }

    if watch {
        println!("Watch mode not yet implemented in headless mode. Use interactive TUI instead.");
    }

    Ok(())
}

fn cmd_plan(
    root: &std::path::Path,
    finding_id: &str,
    copy: bool,
    output: Option<std::path::PathBuf>,
) -> io::Result<()> {
    let mut analyzer = Analyzer::new(root).map_err(|e| io::Error::other(e.to_string()))?;

    let result = analyzer
        .scan()
        .map_err(|e| io::Error::other(e.to_string()))?;

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
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "Config file already exists at {}. Use --force to overwrite.",
                config_path.display()
            ),
        ));
    }

    let config = Config::default();
    config
        .save(&config_path)
        .map_err(|e| io::Error::other(e.to_string()))?;

    println!("Created {}", config_path.display());
    println!();
    println!("Configuration options:");
    println!("  strictness: relaxed | default | strict");
    println!("  exclude: [\"pattern1\", \"pattern2\", ...]");
    println!("  disabled_patterns: [\"god_file\", \"barrel_bloat\", ...]");
    println!();
    println!("See documentation for threshold overrides.");

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

fn cmd_license(action: LicenseAction) -> io::Result<()> {
    let mut manager = LicenseManager::new();

    match action {
        LicenseAction::Status => {
            let status = manager.status();
            println!("Prune License");
            println!("=============");
            println!();
            println!("{}", status.message());

            if let LicenseStatus::Valid(license) | LicenseStatus::GracePeriod(license) = &status {
                println!();
                println!("Tier: {}", license.tier.display_name());
                println!("Email: {}", license.email);
                println!("Features: {}", license.features.join(", "));
                if let Some(seats) = license.seats {
                    println!("Seats: {}", seats);
                }
            }
        }
        LicenseAction::Activate { key, email } => match manager.activate(&key, &email) {
            Ok(license) => {
                println!("License activated successfully!");
                println!();
                println!("Tier: {}", license.tier.display_name());
                println!("Features: {}", license.features.join(", "));
                if license.expires_at.is_none() {
                    println!("License: Perpetual (includes 1 year of updates)");
                } else {
                    println!("Valid for: {} days", license.days_remaining().unwrap_or(0));
                }
            }
            Err(e) => {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, e.to_string()));
            }
        },
        LicenseAction::Trial { email } => match manager.start_trial(&email) {
            Ok(license) => {
                println!("Trial started successfully!");
                println!();
                println!("You have 14 days to explore Prune.");
                println!("Trial key: {}", license.key);
                println!();
                println!("Features included:");
                for feature in &license.features {
                    println!("  - {}", feature);
                }
                println!();
                println!("Purchase at https://prunecode.ai/buy");
            }
            Err(e) => {
                return Err(io::Error::other(e.to_string()));
            }
        },
        LicenseAction::Generate { tier, customer } => {
            let license_tier = match tier.to_lowercase().as_str() {
                "solo" => LicenseTier::Solo,
                "pro" => LicenseTier::Pro,
                "team" => LicenseTier::Team,
                "enterprise" | "ent" => LicenseTier::Enterprise,
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Invalid tier. Use: solo, pro, team, or enterprise",
                    ));
                }
            };

            let key = generate_license_key(license_tier, &customer);
            println!("Generated License Key");
            println!("======================");
            println!();
            println!("Tier: {}", license_tier.display_name());
            println!("Customer: {}", customer);
            println!();
            println!("Key: {}", key);
            println!();
            println!("Activate with:");
            println!("  prune license activate {} --email {}", key, customer);
        }
    }

    Ok(())
}

fn generate_license_key(tier: LicenseTier, identifier: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let tier_code = match tier {
        LicenseTier::Trial => "TRIAL",
        LicenseTier::Solo => "SOLO",
        LicenseTier::Pro => "PRO",
        LicenseTier::Team => "TEAM",
        LicenseTier::Enterprise => "ENT",
    };

    let mut hasher = DefaultHasher::new();
    identifier.hash(&mut hasher);
    chrono::Utc::now()
        .timestamp_nanos_opt()
        .unwrap_or(0)
        .hash(&mut hasher);
    let hash = hasher.finish();

    format!(
        "PRUNE-{}-{:06X}-{:06X}-{:06X}",
        tier_code,
        (hash >> 48) as u32 & 0xFFFFFF,
        (hash >> 24) as u32 & 0xFFFFFF,
        hash as u32 & 0xFFFFFF
    )
}
