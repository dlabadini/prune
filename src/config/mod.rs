//! Configuration management for Prune

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("Failed to read config file: {0}")]
    ReadError(#[from] std::io::Error),
    #[error("Failed to parse config: {0}")]
    ParseError(#[from] toml::de::Error),
    #[error("Failed to serialize config: {0}")]
    SerializeError(#[from] toml::ser::Error),
}

/// Strictness level for pattern detection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Strictness {
    /// Fewer findings, higher thresholds
    Relaxed,
    /// Balanced defaults for small-to-mid teams
    #[default]
    Default,
    /// More findings, lower thresholds
    Strict,
}

impl Strictness {
    pub fn cycle(self) -> Self {
        match self {
            Strictness::Relaxed => Strictness::Default,
            Strictness::Default => Strictness::Strict,
            Strictness::Strict => Strictness::Relaxed,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Strictness::Relaxed => "relaxed",
            Strictness::Default => "default",
            Strictness::Strict => "strict",
        }
    }
}

/// Pattern-specific threshold overrides
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PatternThresholds {
    /// Similarity threshold for interface sprawl detection (0.0-1.0)
    #[serde(default)]
    pub interface_sprawl_similarity: Option<f32>,
    /// Minimum cluster size for interface sprawl
    #[serde(default)]
    pub interface_sprawl_min_cluster: Option<usize>,
    /// Similarity threshold for redundant logic detection
    #[serde(default)]
    pub redundant_logic_similarity: Option<f32>,
    /// Maximum exports before flagging as god file
    #[serde(default)]
    pub god_file_max_exports: Option<usize>,
    /// Maximum lines before flagging as god file
    #[serde(default)]
    pub god_file_max_lines: Option<usize>,
    /// Threshold for barrel bloat detection
    #[serde(default)]
    pub barrel_bloat_threshold: Option<usize>,
    /// Threshold for dead export detection grace period (days)
    #[serde(default)]
    pub dead_export_grace_days: Option<u32>,
}

/// Main configuration struct
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Overall strictness level
    #[serde(default)]
    pub strictness: Strictness,
    /// Pattern-specific threshold overrides
    #[serde(default)]
    pub thresholds: PatternThresholds,
    /// File patterns to exclude (in addition to .gitignore)
    #[serde(default)]
    pub exclude: Vec<String>,
    /// File patterns to include (overrides exclude)
    #[serde(default)]
    pub include: Vec<String>,
    /// Patterns to disable entirely
    #[serde(default)]
    pub disabled_patterns: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            strictness: Strictness::Default,
            thresholds: PatternThresholds::default(),
            exclude: vec![
                "node_modules/**".to_string(),
                "dist/**".to_string(),
                "build/**".to_string(),
                ".git/**".to_string(),
                "*.min.js".to_string(),
                "*.bundle.js".to_string(),
            ],
            include: vec![],
            disabled_patterns: vec![],
        }
    }
}

impl Config {
    /// Load config from file, or return defaults if not found
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            Ok(toml::from_str(&content)?)
        } else {
            Ok(Self::default())
        }
    }

    /// Find and load config from standard locations
    pub fn find_and_load(project_root: &Path) -> Result<Self, ConfigError> {
        // Check for .prunerc first
        let prunerc = project_root.join(".prunerc");
        if prunerc.exists() {
            return Self::load(&prunerc);
        }

        // Check for prune.config.toml
        let config_toml = project_root.join("prune.config.toml");
        if config_toml.exists() {
            return Self::load(&config_toml);
        }

        // Return defaults
        Ok(Self::default())
    }

    /// Save config to file
    pub fn save(&self, path: &Path) -> Result<(), ConfigError> {
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Get effective threshold values based on strictness
    pub fn effective_interface_sprawl_similarity(&self) -> f32 {
        self.thresholds.interface_sprawl_similarity.unwrap_or_else(|| {
            match self.strictness {
                Strictness::Relaxed => 0.90,
                Strictness::Default => 0.85,
                Strictness::Strict => 0.80,
            }
        })
    }

    pub fn effective_interface_sprawl_min_cluster(&self) -> usize {
        self.thresholds.interface_sprawl_min_cluster.unwrap_or_else(|| {
            match self.strictness {
                Strictness::Relaxed => 4,
                Strictness::Default => 3,
                Strictness::Strict => 2,
            }
        })
    }

    pub fn effective_redundant_logic_similarity(&self) -> f32 {
        self.thresholds.redundant_logic_similarity.unwrap_or_else(|| {
            match self.strictness {
                Strictness::Relaxed => 0.90,
                Strictness::Default => 0.85,
                Strictness::Strict => 0.80,
            }
        })
    }

    pub fn effective_god_file_max_exports(&self) -> usize {
        self.thresholds.god_file_max_exports.unwrap_or_else(|| {
            match self.strictness {
                Strictness::Relaxed => 30,
                Strictness::Default => 20,
                Strictness::Strict => 15,
            }
        })
    }

    pub fn effective_god_file_max_lines(&self) -> usize {
        self.thresholds.god_file_max_lines.unwrap_or_else(|| {
            match self.strictness {
                Strictness::Relaxed => 1000,
                Strictness::Default => 500,
                Strictness::Strict => 300,
            }
        })
    }

    pub fn effective_barrel_bloat_threshold(&self) -> usize {
        self.thresholds.barrel_bloat_threshold.unwrap_or_else(|| {
            match self.strictness {
                Strictness::Relaxed => 30,
                Strictness::Default => 20,
                Strictness::Strict => 10,
            }
        })
    }

    /// Check if a pattern is enabled
    pub fn is_pattern_enabled(&self, pattern_name: &str) -> bool {
        !self.disabled_patterns.iter().any(|p| p == pattern_name)
    }

    /// Get the config file path for a project
    pub fn config_path(project_root: &Path) -> PathBuf {
        project_root.join(".prunerc")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strictness_cycle() {
        assert_eq!(Strictness::Relaxed.cycle(), Strictness::Default);
        assert_eq!(Strictness::Default.cycle(), Strictness::Strict);
        assert_eq!(Strictness::Strict.cycle(), Strictness::Relaxed);
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.strictness, Strictness::Default);
        assert!(!config.exclude.is_empty());
    }
}
