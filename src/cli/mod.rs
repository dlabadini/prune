//! Command-line interface for Prune

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "prune")]
#[command(author = "Prune Dev")]
#[command(version = "0.1.0")]
#[command(about = "Detect structural and semantic anti-patterns in codebases", long_about = None)]
pub struct Cli {
    /// Path to the project root (defaults to current directory)
    #[arg(global = true, short, long)]
    pub path: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Scan the codebase for anti-patterns
    Scan {
        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Run in watch mode (headless)
        #[arg(long)]
        watch: bool,

        /// Report only findings missing from a baseline file (default .prune-baseline.json)
        #[arg(long, num_args = 0..=1, default_missing_value = ".prune-baseline.json")]
        baseline: Option<PathBuf>,

        /// Report only findings in files changed since REF (default HEAD) and their dependents
        #[arg(long, num_args = 0..=1, default_missing_value = "HEAD")]
        changed: Option<String>,

        /// Exit with status 1 when any finding is reported (use with --baseline in CI)
        #[arg(long)]
        ci: bool,
    },

    /// Write the current findings to a baseline file so CI reports only new ones
    Baseline {
        /// Baseline file to write
        #[arg(long, short, default_value = ".prune-baseline.json")]
        output: PathBuf,
    },

    /// Apply safe fixes (dry run unless --apply): drop unnecessary `export`, delete unreachable
    /// files. Each applied fix is kept only if the check command passes.
    Fix {
        /// Write changes (default: print the plan)
        #[arg(long)]
        apply: bool,

        /// Check command run after each fix (default: [fix] check in .prunerc)
        #[arg(long)]
        check: Option<String>,

        /// Fix kinds: unexport, delete-file (default: both)
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,

        /// Limit to findings whose files match this path prefix
        #[arg(long)]
        path_prefix: Option<String>,
    },

    /// Rename a symbol across its importers (dry run unless --apply)
    Rename {
        /// File that declares the symbol (repo-relative)
        file: String,
        /// Current name
        from: String,
        /// New name
        to: String,
        /// Write changes (default: print the plan)
        #[arg(long)]
        apply: bool,
        /// Check command run after the rename (default: [fix] check in .prunerc)
        #[arg(long)]
        check: Option<String>,
    },

    /// Generate a refactor plan for a specific finding
    Plan {
        /// Finding ID
        finding_id: String,

        /// Copy plan to clipboard instead of stdout
        #[arg(long)]
        copy: bool,

        /// Save plan to file
        #[arg(long, short)]
        output: Option<PathBuf>,

        /// Full filesystem scan before planning (default: analyze indexed data only)
        #[arg(long)]
        rescan: bool,
    },

    /// Verify a finding is resolved after refactoring
    Verify {
        /// Finding ID (or unique prefix)
        finding_id: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Analyze indexed data only (skip file walk)
        #[arg(long)]
        quick: bool,
    },

    /// Write .prunerc and ignore the local .prune/ directory
    Init {
        /// Force overwrite existing config
        #[arg(long, short)]
        force: bool,
    },

    /// Show configuration
    Config,

    /// Serve findings and refactor plans over MCP stdio
    Mcp,

    /// Manage license
    License {
        #[command(subcommand)]
        action: LicenseAction,
    },
}

#[derive(Subcommand)]
pub enum LicenseAction {
    /// Show license status
    Status,

    /// Activate a license key
    Activate {
        /// License key (format: PRUNE-TIER-XXXXXX-XXXXXX-XXXXXX)
        key: String,

        /// Email address
        #[arg(long, short)]
        email: String,
    },

    /// Start a 14-day trial
    Trial {
        /// Email address
        #[arg(long, short)]
        email: String,
    },

    /// Generate a license key (owner/admin only)
    #[command(hide = true)]
    Generate {
        /// Tier: solo, pro, team, enterprise
        #[arg(long, short)]
        tier: String,

        /// Customer identifier (email or company)
        #[arg(long, short)]
        customer: String,
    },
}

impl Cli {
    pub fn project_root(&self) -> PathBuf {
        self.path
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }
}
