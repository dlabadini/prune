//! TUI Application state and logic

use arboard::Clipboard;
use prune::analyzer::{Analyzer, AnalyzerError, ScanProgress, ScanResult};
use prune::patterns::Finding;
use prune::plan::RefactorPlan;
use prune::score::{self, RepoScore};
use prune::watcher::{FileWatcher, WatchEvent, WatchState};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;

/// Application state enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppState {
    List,
    Detail,
    Help,
}

const COMMANDS: &[(&str, &str)] = &[
    ("strict", "Cycle strictness and rescan"),
    ("watch", "Toggle watch mode"),
    ("sort", "Toggle severity and tier order"),
    ("rescan", "Scan again"),
    ("save", "Write the full plan to REFACTOR_PLAN.md"),
    ("help", "Show keys"),
];

struct ScanDone {
    analyzer: Analyzer,
    result: Result<ScanResult, AnalyzerError>,
}

struct InflightScan {
    done: Receiver<ScanDone>,
    progress: Arc<Mutex<Option<ScanProgress>>>,
}

/// Main application state
pub struct App {
    pub state: AppState,
    pub should_quit: bool,
    pub root: PathBuf,
    analyzer: Option<Analyzer>,
    inflight: Option<InflightScan>,
    pub findings: Vec<Finding>,
    pub filtered_findings: Vec<usize>,
    pub selected_index: usize,
    pub detail_scroll: usize,
    pub query: String,
    pub sort_by_tier: bool,
    pub score: Option<RepoScore>,
    skipped: HashSet<String>,
    pub status_message: Option<String>,
    pub last_scan: Option<ScanResult>,
    pub watch_state: WatchState,
    file_watcher: Option<FileWatcher>,
    clipboard: Option<Clipboard>,
}

impl App {
    pub fn new(root: &Path) -> Result<Self, AnalyzerError> {
        let analyzer = Analyzer::new(root)?;
        let clipboard = Clipboard::new().ok();

        Ok(Self {
            state: AppState::List,
            should_quit: false,
            root: root.to_path_buf(),
            analyzer: Some(analyzer),
            inflight: None,
            findings: Vec::new(),
            filtered_findings: Vec::new(),
            selected_index: 0,
            detail_scroll: 0,
            query: String::new(),
            sort_by_tier: false,
            score: None,
            skipped: HashSet::new(),
            status_message: None,
            last_scan: None,
            watch_state: WatchState::new(),
            file_watcher: None,
            clipboard,
        })
    }

    pub fn scanning(&self) -> bool {
        self.inflight.is_some()
    }

    /// Start a scan on a background thread so the status line can update.
    pub fn start_scan(&mut self) {
        if self.inflight.is_some() {
            return;
        }
        let Some(mut analyzer) = self.analyzer.take() else {
            return;
        };
        let model_ready = prune::embeddings::embedding_model_ready();
        self.set_status(if model_ready {
            "Scanning...".to_string()
        } else {
            "Downloading local embedding model (~80 MB), then scanning".to_string()
        });

        let progress = Arc::new(Mutex::new(None));
        let slot = Arc::clone(&progress);
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let result = analyzer.scan_reporting(&mut |event| {
                if let Ok(mut current) = slot.lock() {
                    *current = Some(event);
                }
            });
            let _ = tx.send(ScanDone { analyzer, result });
        });
        self.inflight = Some(InflightScan { done: rx, progress });
    }

    /// Pull progress and apply a finished scan.
    pub fn poll_scan(&mut self) {
        let finished = self
            .inflight
            .as_ref()
            .and_then(|inflight| inflight.done.try_recv().ok());
        if let Some(done) = finished {
            self.inflight = None;
            self.analyzer = Some(done.analyzer);
            match done.result {
                Ok(result) => {
                    self.skipped.clear();
                    self.findings = result.findings.clone();
                    self.score = Some(result.score.clone());
                    self.last_scan = Some(result.clone());
                    self.apply_filter();
                    self.set_status(format!(
                        "{}  ·  {} files in {}ms",
                        result.score.summary(),
                        result.files_scanned,
                        result.scan_duration_ms
                    ));
                }
                Err(error) => self.set_status(format!("Scan error: {error}")),
            }
            return;
        }

        let message = self.inflight.as_ref().and_then(|inflight| {
            inflight
                .progress
                .lock()
                .ok()
                .and_then(|slot| slot.clone())
                .map(progress_label)
        });
        if let Some(message) = message {
            self.status_message = Some(message);
        }
    }

    /// Re-index a single file
    pub fn reindex_file(&mut self, path: &Path) {
        let Some(analyzer) = self.analyzer.as_mut() else {
            return;
        };
        if let Err(error) = analyzer.reindex_file(path) {
            self.set_status(format!("Reindex error: {error}"));
        }
    }

    /// Refresh findings without a full file walk.
    pub fn refresh_findings(&mut self) {
        let Some(analyzer) = self.analyzer.as_ref() else {
            return;
        };
        self.findings = analyzer.get_findings();
        self.score = analyzer.score_findings(&self.findings).ok();
        self.apply_filter();
    }

    fn apply_filter(&mut self) {
        let needle = if self.query.starts_with('/') {
            String::new()
        } else {
            self.query.to_lowercase()
        };
        let mut indices: Vec<usize> = self
            .findings
            .iter()
            .enumerate()
            .filter(|(_, finding)| !self.skipped.contains(&finding.id))
            .filter(|(_, finding)| {
                if needle.is_empty() {
                    return true;
                }
                let title = finding.title.to_lowercase();
                let pattern = finding.pattern_type.display_name().to_lowercase();
                let paths = finding
                    .locations
                    .iter()
                    .map(|location| location.file.to_lowercase())
                    .collect::<Vec<_>>()
                    .join(" ");
                title.contains(&needle) || pattern.contains(&needle) || paths.contains(&needle)
            })
            .map(|(index, _)| index)
            .collect();

        if self.sort_by_tier {
            indices.sort_by(|left, right| {
                let a = &self.findings[*left];
                let b = &self.findings[*right];
                a.pattern_type
                    .tier()
                    .cmp(&b.pattern_type.tier())
                    .then(b.severity.cmp(&a.severity))
            });
        }

        self.filtered_findings = indices;
        if self.selected_index >= self.filtered_findings.len() {
            self.selected_index = 0;
        }
    }

    pub fn selected_finding(&self) -> Option<&Finding> {
        self.filtered_findings
            .get(self.selected_index)
            .and_then(|&idx| self.findings.get(idx))
    }

    pub fn matching_commands(&self) -> Vec<(&'static str, &'static str)> {
        let Some(typed) = self.query.strip_prefix('/') else {
            return Vec::new();
        };
        let typed = typed.trim();
        COMMANDS
            .iter()
            .copied()
            .filter(|(name, _)| typed.is_empty() || name.starts_with(typed))
            .collect()
    }

    pub fn push_query(&mut self, ch: char) {
        if self.scanning() {
            return;
        }
        self.query.push(ch);
        self.apply_filter();
    }

    pub fn pop_query(&mut self) {
        self.query.pop();
        self.apply_filter();
    }

    pub fn clear_query(&mut self) {
        self.query.clear();
        self.apply_filter();
    }

    /// Run a `/command` when the typed name is exact or a unique prefix.
    pub fn run_command(&mut self) {
        let Some(typed) = self.query.strip_prefix('/') else {
            return;
        };
        let typed = typed.trim();
        if typed.is_empty() {
            return;
        }
        let matches = self.matching_commands();
        let name = if let Some((exact, _)) = matches.iter().find(|(name, _)| *name == typed) {
            *exact
        } else if matches.len() == 1 {
            matches[0].0
        } else {
            self.set_status("Unknown command. Try /rescan, /strict, /watch, /sort, /save".into());
            return;
        };
        self.query.clear();
        self.apply_filter();
        match name {
            "strict" => self.cycle_strictness(),
            "watch" => self.toggle_watch_mode(),
            "sort" => {
                self.sort_by_tier = !self.sort_by_tier;
                self.apply_filter();
                self.set_status(if self.sort_by_tier {
                    "Sorted by tier".into()
                } else {
                    "Sorted by severity".into()
                });
            }
            "rescan" => self.start_scan(),
            "save" => self.save_plan_to_file(),
            "help" => self.state = AppState::Help,
            _ => {}
        }
    }

    pub fn next_finding(&mut self) {
        if self.selected_index < self.filtered_findings.len().saturating_sub(1) {
            self.selected_index += 1;
        }
    }

    pub fn previous_finding(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub fn first_finding(&mut self) {
        self.selected_index = 0;
    }

    pub fn last_finding(&mut self) {
        self.selected_index = self.filtered_findings.len().saturating_sub(1);
    }

    pub fn enter_detail(&mut self) {
        if self.selected_finding().is_some() {
            self.state = AppState::Detail;
            self.detail_scroll = 0;
        }
    }

    pub fn exit_detail(&mut self) {
        self.state = AppState::List;
    }

    pub fn scroll_detail_up(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_sub(1);
    }

    pub fn scroll_detail_down(&mut self) {
        self.detail_scroll += 1;
    }

    pub fn scroll_detail_page_up(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_sub(10);
    }

    pub fn scroll_detail_page_down(&mut self) {
        self.detail_scroll += 10;
    }

    /// Hide the selected finding until the next scan. The score stays put.
    pub fn skip_selected(&mut self) {
        if let Some(id) = self.selected_finding().map(|finding| finding.id.clone()) {
            self.skipped.insert(id);
            self.apply_filter();
            self.set_status("Skipped for this session".to_string());
        }
    }

    /// Copy the one-step prompt, not the full plan.
    pub fn copy_next_step(&mut self) {
        let Some(finding) = self.selected_finding() else {
            return;
        };
        let prompt = score::action_prompt(finding);
        if let Some(clipboard) = self.clipboard.as_mut() {
            match clipboard.set_text(prompt) {
                Ok(()) => self.set_status("Next step copied".to_string()),
                Err(error) => self.set_status(format!("Clipboard error: {error}")),
            }
        } else {
            self.set_status("Clipboard not available".to_string());
        }
    }

    pub fn save_plan_to_file(&mut self) {
        if let Some(finding) = self.selected_finding() {
            let plan = RefactorPlan::generate(finding);
            let path = self.root.join("REFACTOR_PLAN.md");
            match plan.write_to_file(&path) {
                Ok(()) => self.set_status(format!("Plan saved to {}", path.display())),
                Err(error) => self.set_status(format!("Save error: {error}")),
            }
        }
    }

    pub fn toggle_watch_mode(&mut self) {
        if self.watch_state.is_watching {
            self.stop_watching();
        } else {
            self.start_watching();
        }
    }

    fn start_watching(&mut self) {
        match FileWatcher::new(&self.root, 200) {
            Ok(mut watcher) => {
                if let Err(error) = watcher.start() {
                    self.set_status(format!("Watch error: {error}"));
                    return;
                }
                self.file_watcher = Some(watcher);
                self.watch_state.start();
                self.set_status("Watch mode enabled".to_string());
            }
            Err(error) => self.set_status(format!("Watch error: {error}")),
        }
    }

    fn stop_watching(&mut self) {
        if let Some(mut watcher) = self.file_watcher.take() {
            let _ = watcher.stop();
        }
        self.watch_state.stop();
        self.set_status("Watch mode disabled".to_string());
    }

    pub fn check_file_events(&mut self) -> Option<Vec<WatchEvent>> {
        self.file_watcher.as_ref()?.try_recv()
    }

    pub fn cycle_strictness(&mut self) {
        let Some(analyzer) = self.analyzer.as_mut() else {
            self.set_status("Wait for the scan to finish".to_string());
            return;
        };
        let new_strictness = analyzer.config().strictness.cycle();
        analyzer.config_mut().strictness = new_strictness;
        self.set_status(format!("Strictness: {}", new_strictness.as_str()));
        self.start_scan();
    }

    pub fn set_status(&mut self, message: String) {
        self.status_message = Some(message);
    }
}

fn progress_label(progress: ScanProgress) -> String {
    match progress {
        ScanProgress::Indexing { done, total, file } => {
            format!("Reading {file} ({done}/{total})")
        }
        ScanProgress::Embedding { model_ready } => {
            if model_ready {
                "Embedding symbols".to_string()
            } else {
                "Downloading local embedding model (~80 MB)".to_string()
            }
        }
        ScanProgress::Scoring => "Scoring findings".to_string(),
    }
}
