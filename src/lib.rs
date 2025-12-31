//! Prune Core Library
//!
//! Detects structural and semantic anti-patterns in codebases,
//! particularly those introduced by AI coding assistants.

pub mod analyzer;
pub mod config;
pub mod db;
pub mod license;
pub mod patterns;
pub mod plan;
pub mod watcher;

pub use analyzer::{Analyzer, AnalyzerError, ScanResult};
pub use config::Config;
pub use db::Database;
pub use license::{License, LicenseError, LicenseManager, LicenseStatus, LicenseTier};
pub use patterns::{Finding, PatternDetector, PatternType, Severity};
pub use plan::RefactorPlan;
