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
