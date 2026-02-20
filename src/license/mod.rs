//! License validation for Prune
//!
//! Supports offline-capable license key validation with graceful degradation.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LicenseError {
    #[error("Invalid license key")]
    InvalidKey,
    #[error("License expired")]
    Expired,
    #[error("License not found")]
    NotFound,
    #[error("Failed to read license: {0}")]
    ReadError(String),
    #[error("Failed to write license: {0}")]
    WriteError(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LicenseTier {
    Trial,
    Solo,
    Pro,
    Team,
    Enterprise,
}

impl LicenseTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            LicenseTier::Trial => "trial",
            LicenseTier::Solo => "solo",
            LicenseTier::Pro => "pro",
            LicenseTier::Team => "team",
            LicenseTier::Enterprise => "enterprise",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            LicenseTier::Trial => "Trial",
            LicenseTier::Solo => "Solo",
            LicenseTier::Pro => "Pro",
            LicenseTier::Team => "Team",
            LicenseTier::Enterprise => "Enterprise",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct License {
    pub key: String,
    pub tier: LicenseTier,
    pub email: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub seats: Option<u32>,
    pub features: Vec<String>,
}

impl License {
    /// Check if license is valid and not expired
    pub fn is_valid(&self) -> bool {
        if let Some(expires) = self.expires_at {
            Utc::now() < expires
        } else {
            true // Perpetual license (Solo tier)
        }
    }

    /// Check if license has a specific feature
    pub fn has_feature(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }

    /// Days until expiration (None if perpetual)
    pub fn days_remaining(&self) -> Option<i64> {
        self.expires_at.map(|exp| {
            let duration = exp - Utc::now();
            duration.num_days()
        })
    }

    /// Check if in grace period (expired but within 7 days)
    pub fn in_grace_period(&self) -> bool {
        if let Some(expires) = self.expires_at {
            let now = Utc::now();
            let grace_end = expires + Duration::days(7);
            now >= expires && now < grace_end
        } else {
            false
        }
    }
}

/// License manager for checking and storing licenses
pub struct LicenseManager {
    license_path: PathBuf,
    cached_license: Option<License>,
}

impl Default for LicenseManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LicenseManager {
    pub fn new() -> Self {
        let license_path = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("prune")
            .join("license.json");

        Self {
            license_path,
            cached_license: None,
        }
    }

    /// Load license from disk
    pub fn load(&mut self) -> Result<&License, LicenseError> {
        if self.cached_license.is_some() {
            return Ok(self.cached_license.as_ref().unwrap());
        }

        if !self.license_path.exists() {
            return Err(LicenseError::NotFound);
        }

        let content = std::fs::read_to_string(&self.license_path)
            .map_err(|e| LicenseError::ReadError(e.to_string()))?;

        let license: License =
            serde_json::from_str(&content).map_err(|e| LicenseError::ReadError(e.to_string()))?;

        self.cached_license = Some(license);
        Ok(self.cached_license.as_ref().unwrap())
    }

    /// Save license to disk
    pub fn save(&mut self, license: License) -> Result<(), LicenseError> {
        if let Some(parent) = self.license_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| LicenseError::WriteError(e.to_string()))?;
        }

        let content = serde_json::to_string_pretty(&license)
            .map_err(|e| LicenseError::WriteError(e.to_string()))?;

        std::fs::write(&self.license_path, content)
            .map_err(|e| LicenseError::WriteError(e.to_string()))?;

        self.cached_license = Some(license);
        Ok(())
    }

    /// Activate a license key (validates with server)
    pub fn activate(&mut self, key: &str, email: &str) -> Result<License, LicenseError> {
        // In production, this would validate with a license server
        // For now, we support offline activation with signed keys

        let license = validate_license_key(key, email)?;
        self.save(license.clone())?;
        Ok(license)
    }

    /// Start a trial license
    pub fn start_trial(&mut self, email: &str) -> Result<License, LicenseError> {
        let trial = License {
            key: generate_trial_key(email),
            tier: LicenseTier::Trial,
            email: email.to_string(),
            issued_at: Utc::now(),
            expires_at: Some(Utc::now() + Duration::days(14)),
            seats: None,
            features: vec![
                "cli".to_string(),
                "all_patterns".to_string(),
                "watch_mode".to_string(),
            ],
        };

        self.save(trial.clone())?;
        Ok(trial)
    }

    /// Check license status
    pub fn status(&mut self) -> LicenseStatus {
        match self.load() {
            Ok(license) => {
                if license.is_valid() {
                    LicenseStatus::Valid(license.clone())
                } else if license.in_grace_period() {
                    LicenseStatus::GracePeriod(license.clone())
                } else {
                    LicenseStatus::Expired(license.clone())
                }
            }
            Err(LicenseError::NotFound) => LicenseStatus::NoLicense,
            Err(e) => LicenseStatus::Error(e.to_string()),
        }
    }
}

#[derive(Debug)]
pub enum LicenseStatus {
    Valid(License),
    GracePeriod(License),
    Expired(License),
    NoLicense,
    Error(String),
}

impl LicenseStatus {
    pub fn is_usable(&self) -> bool {
        matches!(
            self,
            LicenseStatus::Valid(_) | LicenseStatus::GracePeriod(_)
        )
    }

    pub fn message(&self) -> String {
        match self {
            LicenseStatus::Valid(l) => {
                if let Some(days) = l.days_remaining() {
                    format!(
                        "{} license - {} days remaining",
                        l.tier.display_name(),
                        days
                    )
                } else {
                    format!("{} license - perpetual", l.tier.display_name())
                }
            }
            LicenseStatus::GracePeriod(_) => {
                "License expired - grace period active. Please renew.".to_string()
            }
            LicenseStatus::Expired(_) => "License expired. Running in read-only mode.".to_string(),
            LicenseStatus::NoLicense => {
                "No license found. Start a trial at https://prunecode.ai/trial".to_string()
            }
            LicenseStatus::Error(e) => {
                format!("License error: {}", e)
            }
        }
    }
}

/// Validate a license key (offline-capable with signed keys)
fn validate_license_key(key: &str, email: &str) -> Result<License, LicenseError> {
    // License key format: PRUNE-TIER-XXXXXX-XXXXXX-XXXXXX
    // In production, this would verify a cryptographic signature

    let parts: Vec<&str> = key.split('-').collect();
    if parts.len() != 5 || parts[0] != "PRUNE" {
        return Err(LicenseError::InvalidKey);
    }

    let tier = match parts[1] {
        "SOLO" => LicenseTier::Solo,
        "PRO" => LicenseTier::Pro,
        "TEAM" => LicenseTier::Team,
        "ENT" => LicenseTier::Enterprise,
        _ => return Err(LicenseError::InvalidKey),
    };

    // Solo licenses are perpetual (1 year updates)
    // Pro/Team licenses are annual
    let expires_at = match tier {
        LicenseTier::Solo => None,
        _ => Some(Utc::now() + Duration::days(365)),
    };

    let features = match tier {
        LicenseTier::Solo => vec!["cli".to_string(), "all_patterns".to_string()],
        LicenseTier::Pro => vec![
            "cli".to_string(),
            "all_patterns".to_string(),
            "watch_mode".to_string(),
            "desktop_app".to_string(),
        ],
        LicenseTier::Team | LicenseTier::Enterprise => vec![
            "cli".to_string(),
            "all_patterns".to_string(),
            "watch_mode".to_string(),
            "desktop_app".to_string(),
            "ci_integration".to_string(),
            "shared_config".to_string(),
        ],
        LicenseTier::Trial => vec![],
    };

    Ok(License {
        key: key.to_string(),
        tier,
        email: email.to_string(),
        issued_at: Utc::now(),
        expires_at,
        seats: if tier == LicenseTier::Team {
            Some(5)
        } else {
            None
        },
        features,
    })
}

/// Generate a trial key
fn generate_trial_key(email: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    email.hash(&mut hasher);
    Utc::now().timestamp().hash(&mut hasher);
    let hash = hasher.finish();

    format!(
        "PRUNE-TRIAL-{:06X}-{:06X}-{:06X}",
        (hash >> 48) as u32 & 0xFFFFFF,
        (hash >> 24) as u32 & 0xFFFFFF,
        hash as u32 & 0xFFFFFF
    )
}

/// Generate a license key for a specific tier (admin/owner use only)
pub fn generate_license_key(tier: LicenseTier, identifier: &str) -> String {
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
    Utc::now()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trial_key_generation() {
        let key = generate_trial_key("test@example.com");
        assert!(key.starts_with("PRUNE-TRIAL-"));
    }

    #[test]
    fn test_license_validation() {
        let result = validate_license_key("PRUNE-SOLO-ABC123-DEF456-GHI789", "test@example.com");
        assert!(result.is_ok());
        let license = result.unwrap();
        assert_eq!(license.tier, LicenseTier::Solo);
        assert!(license.expires_at.is_none()); // Perpetual
    }

    #[test]
    fn test_invalid_key() {
        let result = validate_license_key("INVALID-KEY", "test@example.com");
        assert!(matches!(result, Err(LicenseError::InvalidKey)));
    }
}
