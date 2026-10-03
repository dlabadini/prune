//! Repo score: essential and recommended pools, plus a capped emerging bonus.
//!
//! A pattern that does not apply is left out of its pool. A partial finding earns
//! half of that check's share. Emerging findings can only add bonus points.

use crate::config::Config;
use crate::db::{Database, SymbolKind};
use crate::patterns::{Finding, PatternType, Severity, Tier};
use serde::Serialize;

const ESSENTIAL_POOL: f32 = 80.0;
const RECOMMENDED_POOL: f32 = 20.0;
const BONUS_CAP: f32 = 5.0;

/// How a single eligible check turned out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Pass,
    Partial,
    Fail,
}

impl Outcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            Outcome::Pass => "pass",
            Outcome::Partial => "partial",
            Outcome::Fail => "fail",
        }
    }

    fn credit(self) -> f32 {
        match self {
            Outcome::Pass => 1.0,
            Outcome::Partial => 0.5,
            Outcome::Fail => 0.0,
        }
    }
}

/// One eligible check after applicability has been decided.
#[derive(Debug, Clone, Copy)]
pub struct ScoredCheck {
    pub tier: Tier,
    pub outcome: Outcome,
}

/// The number a rescan can be compared against.
#[derive(Debug, Clone, Serialize)]
pub struct RepoScore {
    pub total: f32,
    pub label: String,
    pub essential_earned: f32,
    pub essential_available: f32,
    pub recommended_earned: f32,
    pub recommended_available: f32,
    pub bonus: f32,
    pub version: String,
}

impl RepoScore {
    pub fn summary(&self) -> String {
        format!("{:.0}  {}", self.total, self.label)
    }
}

/// Id that stays the same for the same pattern, title, and locations.
pub fn stable_finding_id(pattern: PatternType, keys: &[String]) -> String {
    let mut keys = keys.to_vec();
    keys.sort();
    let key = format!("{}\n{}", pattern.as_str(), keys.join("\n"));
    format!("{}-{:016x}", pattern.as_str(), fnv64(&key))
}

/// Fail when the finding is high or critical. Anything milder is partial credit.
pub fn finding_outcome(finding: &Finding) -> Outcome {
    if finding.severity >= Severity::High {
        Outcome::Fail
    } else {
        Outcome::Partial
    }
}

/// One copyable prompt: what was observed, the change, and the rescan check.
pub fn action_prompt(finding: &Finding) -> String {
    let places: Vec<String> = finding
        .locations
        .iter()
        .map(|location| format!("{}:{}", location.file, location.start_line))
        .collect();
    let observed = finding
        .locations
        .iter()
        .find_map(|location| location.snippet.as_ref().map(|snippet| first_line(snippet)))
        .unwrap_or_else(|| first_line(&finding.description));
    let next = if !finding.action_summary.is_empty() {
        finding.action_summary.clone()
    } else if finding.suggested_fix.trim().is_empty() {
        first_line(&finding.description)
    } else {
        first_line(&finding.suggested_fix)
    };
    format!(
        "{pattern} ({tier}, {outcome})\n{places}\nObserved: {observed}\nNext: {next}\nRescan: finding `{id}` should be gone.\n",
        pattern = finding.pattern_type.display_name(),
        tier = finding.pattern_type.tier().as_str(),
        outcome = finding_outcome(finding).as_str(),
        places = if places.is_empty() {
            "(no location)".to_string()
        } else {
            places.join("\n")
        },
        observed = observed,
        next = next,
        id = finding.id,
    )
}

pub fn evidence_line(finding: &Finding) -> String {
    if let Some(location) = finding.locations.first() {
        let snippet = location
            .snippet
            .as_deref()
            .or(location.context.as_deref())
            .map(first_line);
        match snippet {
            Some(text) if !text.is_empty() => {
                format!("{}:{}  {text}", location.file, location.start_line)
            }
            _ => format!("{}:{}", location.file, location.start_line),
        }
    } else {
        first_line(&finding.description)
    }
}

pub fn next_line(finding: &Finding) -> String {
    if finding.suggested_fix.trim().is_empty() {
        first_line(&finding.description)
    } else {
        first_step(&finding.suggested_fix)
    }
}

/// First line of a fix that says what to do: skips a heading such as "In `src/a.ts`:".
fn first_step(text: &str) -> String {
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let first = lines.next().unwrap_or("");
    match lines.next() {
        Some(step) if first.ends_with(':') => {
            let step = step.trim_start_matches(['-', '*', ' ']);
            format!("{} {step}", first.trim_end_matches(':'))
        }
        _ => first.to_string(),
    }
}

/// Score the findings currently in the index.
pub fn score_repo(
    db: &Database,
    config: &Config,
    findings: &[Finding],
) -> Result<RepoScore, crate::db::DbError> {
    let mut checks = Vec::new();
    let mut emerging_findings = 0usize;

    for pattern in PatternType::all() {
        if !config.is_pattern_enabled(pattern.as_str()) || !pattern_applies(db, *pattern)? {
            continue;
        }
        let matched: Vec<&Finding> = findings
            .iter()
            .filter(|finding| finding.pattern_type == *pattern)
            .collect();
        if pattern.tier() == Tier::Emerging {
            emerging_findings = matched.len();
            continue;
        }
        let outcome = if matched.is_empty() {
            Outcome::Pass
        } else if matched
            .iter()
            .any(|finding| finding.severity >= Severity::High)
        {
            Outcome::Fail
        } else {
            Outcome::Partial
        };
        checks.push(ScoredCheck {
            tier: pattern.tier(),
            outcome,
        });
    }

    Ok(tally(&checks, emerging_findings))
}

/// Pure pool math. Ineligible checks are omitted by the caller.
pub fn tally(checks: &[ScoredCheck], emerging_findings: usize) -> RepoScore {
    let essential: Vec<_> = checks
        .iter()
        .filter(|check| check.tier == Tier::Essential)
        .collect();
    let recommended: Vec<_> = checks
        .iter()
        .filter(|check| check.tier == Tier::Recommended)
        .collect();

    let (essential_earned, essential_available) = pool(&essential, ESSENTIAL_POOL);
    let (recommended_earned, recommended_available) = pool(&recommended, RECOMMENDED_POOL);
    let bonus = (emerging_findings as f32).min(BONUS_CAP);
    let total = essential_earned + recommended_earned + bonus;

    RepoScore {
        total,
        label: label_for(total, checks.is_empty() && emerging_findings == 0).to_string(),
        essential_earned,
        essential_available,
        recommended_earned,
        recommended_available,
        bonus,
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn pool(checks: &[&ScoredCheck], points: f32) -> (f32, f32) {
    if checks.is_empty() {
        return (0.0, 0.0);
    }
    let share = points / checks.len() as f32;
    let earned = checks
        .iter()
        .map(|check| share * check.outcome.credit())
        .sum();
    (earned, points)
}

fn label_for(total: f32, nothing_to_score: bool) -> &'static str {
    if nothing_to_score {
        "Nothing to score"
    } else if total >= 90.0 {
        "Clear"
    } else if total >= 70.0 {
        "A few gaps"
    } else if total >= 50.0 {
        "Blockers remain"
    } else {
        "Important blockers remain"
    }
}

fn pattern_applies(db: &Database, pattern: PatternType) -> Result<bool, crate::db::DbError> {
    let applies = match pattern {
        PatternType::GodFile => !db.get_all_files()?.is_empty(),
        PatternType::DeadExports => !db.get_exported_symbols()?.is_empty(),
        PatternType::ImportCycle => !db.import_edges()?.is_empty(),
        PatternType::InterfaceSprawl | PatternType::Lexicon => {
            !db.get_symbols_by_kind(SymbolKind::Interface)?.is_empty()
                || !db.get_symbols_by_kind(SymbolKind::TypeAlias)?.is_empty()
        }
        PatternType::RedundantLogic | PatternType::OverAbstraction => {
            !db.get_symbols_by_kind(SymbolKind::Function)?.is_empty()
        }
        PatternType::BarrelBloat => !db.get_reexport_counts()?.is_empty(),
    };
    Ok(applies)
}

fn first_line(text: &str) -> String {
    text.lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim()
        .to_string()
}

fn fnv64(text: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_id_ignores_key_order() {
        let first = stable_finding_id(
            PatternType::DeadExports,
            &["symbol:b.ts::y".into(), "symbol:a.ts::x".into()],
        );
        let second = stable_finding_id(
            PatternType::DeadExports,
            &["symbol:a.ts::x".into(), "symbol:b.ts::y".into()],
        );
        assert_eq!(first, second);
        assert!(first.starts_with("dead_exports-"));
    }

    #[test]
    fn finding_id_ignores_title_and_lines() {
        use crate::patterns::{Finding, Location, Severity};
        let mut first = Finding::new(PatternType::GodFile, Severity::High, 0.9, "40 exports".into(), "d".into())
            .with_locations(vec![Location::new("a.ts".into(), 1, 400)]);
        let mut second = Finding::new(PatternType::GodFile, Severity::High, 0.9, "41 exports".into(), "d".into())
            .with_locations(vec![Location::new("a.ts".into(), 1, 420)]);
        first.stabilize_id();
        second.stabilize_id();
        assert_eq!(first.id, second.id);
    }

    #[test]
    fn ineligible_checks_leave_the_pool_and_bonus_cannot_go_negative() {
        let score = tally(
            &[ScoredCheck {
                tier: Tier::Essential,
                outcome: Outcome::Pass,
            }],
            0,
        );
        assert!((score.essential_earned - 80.0).abs() < 0.01);
        assert!((score.recommended_available - 0.0).abs() < 0.01);
        assert_eq!(score.label, "A few gaps");

        let failed = tally(
            &[ScoredCheck {
                tier: Tier::Essential,
                outcome: Outcome::Fail,
            }],
            9,
        );
        assert!((failed.total - 5.0).abs() < 0.01);
        assert!(failed.bonus <= 5.0);

        let partial = tally(
            &[
                ScoredCheck {
                    tier: Tier::Essential,
                    outcome: Outcome::Partial,
                },
                ScoredCheck {
                    tier: Tier::Essential,
                    outcome: Outcome::Pass,
                },
            ],
            0,
        );
        assert!((partial.essential_earned - 60.0).abs() < 0.01);
    }
}
