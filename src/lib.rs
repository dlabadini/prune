//! Prune Core Library
//!
//! Detects structural and semantic anti-patterns in codebases,
//! particularly those introduced by AI coding assistants.

pub mod analyzer;
pub mod config;
pub mod db;
pub mod embeddings;
pub mod gitinfo;
pub mod license;
pub mod mcp;
pub mod patterns;
pub mod plan;
pub mod score;
pub mod structural;
pub mod watcher;

pub use analyzer::{Analyzer, AnalyzerError, ScanResult};
pub use config::Config;
pub use db::Database;
pub use license::{
    generate_license_key, License, LicenseError, LicenseManager, LicenseStatus, LicenseTier,
};
pub use patterns::{Finding, PatternDetector, PatternType, Severity, Tier};
pub use plan::RefactorPlan;
pub use score::{action_prompt, RepoScore};
