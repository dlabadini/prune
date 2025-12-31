//! TUI Application state and logic

use prune_core::analyzer::{Analyzer, AnalyzerError, ScanResult};
use prune_core::config::Strictness;
use prune_core::patterns::{Finding, PatternType};
use prune_core::plan::RefactorPlan;
use prune_core::watcher::{FileWatcher, WatchEvent, WatchState};
use arboard::Clipboard;
use std::path::{Path, PathBuf};

/// Application state enum
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppState {
    List,
    Detail,
    Help,
}

/// Filter for findings
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingFilter {
    All,
    Pattern(PatternType),
}

impl FindingFilter {
    pub fn cycle(self) -> Self {
        match self {
            FindingFilter::All => FindingFilter::Pattern(PatternType::InterfaceSprawl),
            FindingFilter::Pattern(PatternType::InterfaceSprawl) => FindingFilter::Pattern(PatternType::RedundantLogic),
            FindingFilter::Pattern(PatternType::RedundantLogic) => FindingFilter::Pattern(PatternType::GodFile),
            FindingFilter::Pattern(PatternType::GodFile) => FindingFilter::Pattern(PatternType::BarrelBloat),
            FindingFilter::Pattern(PatternType::BarrelBloat) => FindingFilter::Pattern(PatternType::DeadExports),
            FindingFilter::Pattern(PatternType::DeadExports) => FindingFilter::Pattern(PatternType::OverAbstraction),
            FindingFilter::Pattern(PatternType::OverAbstraction) => FindingFilter::All,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            FindingFilter::All => "All",
            FindingFilter::Pattern(p) => p.display_name(),
        }
    }
}

/// Main application state
pub struct App {
    pub state: AppState,
    pub should_quit: bool,
    pub root: PathBuf,
    pub analyzer: Analyzer,
    pub findings: Vec<Finding>,
    pub filtered_findings: Vec<usize>, // Indices into findings
    pub selected_index: usize,
    pub detail_scroll: usize,
    pub filter: FindingFilter,
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
            analyzer,
            findings: Vec::new(),
            filtered_findings: Vec::new(),
            selected_index: 0,
            detail_scroll: 0,
            filter: FindingFilter::All,
            status_message: None,
            last_scan: None,
            watch_state: WatchState::new(),
            file_watcher: None,
            clipboard,
        })
    }

    /// Run initial scan
    pub fn scan(&mut self) {
        self.set_status("Scanning...".to_string());

        match self.analyzer.scan() {
            Ok(result) => {
                self.findings = result.findings.clone();
                self.last_scan = Some(result.clone());
                self.apply_filter();
                self.set_status(format!(
                    "Scanned {} files in {}ms. Found {} issues.",
                    result.files_scanned,
                    result.scan_duration_ms,
                    self.findings.len()
                ));
            }
            Err(e) => {
                self.set_status(format!("Scan error: {}", e));
            }
        }
    }

    /// Refresh findings without full rescan
    pub fn refresh_findings(&mut self) {
        self.findings = self.analyzer.get_findings();
        self.apply_filter();
    }

    /// Re-index a single file
    pub fn reindex_file(&mut self, path: &Path) {
        if let Err(e) = self.analyzer.reindex_file(path) {
            self.set_status(format!("Reindex error: {}", e));
        }
    }

    /// Apply current filter to findings
    fn apply_filter(&mut self) {
        self.filtered_findings = self.findings
            .iter()
            .enumerate()
            .filter(|(_, f)| match self.filter {
                FindingFilter::All => true,
                FindingFilter::Pattern(p) => f.pattern_type == p,
            })
            .map(|(i, _)| i)
            .collect();

        // Reset selection if out of bounds
        if self.selected_index >= self.filtered_findings.len() {
            self.selected_index = 0;
        }
    }

    /// Get the currently selected finding
    pub fn selected_finding(&self) -> Option<&Finding> {
        self.filtered_findings.get(self.selected_index)
            .and_then(|&idx| self.findings.get(idx))
    }

    /// Navigation
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

    /// Enter detail view for selected finding
    pub fn enter_detail(&mut self) {
        if self.selected_finding().is_some() {
            self.state = AppState::Detail;
            self.detail_scroll = 0;
        }
    }

    /// Exit detail view
    pub fn exit_detail(&mut self) {
        self.state = AppState::List;
    }

    /// Scroll in detail view
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

    /// Copy refactor plan to clipboard
    pub fn copy_plan_to_clipboard(&mut self) {
        if let Some(finding) = self.selected_finding() {
            let plan = RefactorPlan::generate(finding);

            if let Some(ref mut clipboard) = self.clipboard {
                match clipboard.set_text(plan.as_markdown()) {
                    Ok(_) => self.set_status("Plan copied to clipboard".to_string()),
                    Err(e) => self.set_status(format!("Clipboard error: {}", e)),
                }
            } else {
                self.set_status("Clipboard not available".to_string());
            }
        }
    }

    /// Save refactor plan to file
    pub fn save_plan_to_file(&mut self) {
        if let Some(finding) = self.selected_finding() {
            let plan = RefactorPlan::generate(finding);
            let path = self.root.join("REFACTOR_PLAN.md");

            match plan.write_to_file(&path) {
                Ok(_) => self.set_status(format!("Plan saved to {}", path.display())),
                Err(e) => self.set_status(format!("Save error: {}", e)),
            }
        }
    }

    /// Toggle watch mode
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
                if let Err(e) = watcher.start() {
                    self.set_status(format!("Watch error: {}", e));
                    return;
                }
                self.file_watcher = Some(watcher);
                self.watch_state.start();
                self.set_status("Watch mode enabled".to_string());
            }
            Err(e) => {
                self.set_status(format!("Watch error: {}", e));
            }
        }
    }

    fn stop_watching(&mut self) {
        if let Some(mut watcher) = self.file_watcher.take() {
            let _ = watcher.stop();
        }
        self.watch_state.stop();
        self.set_status("Watch mode disabled".to_string());
    }

    /// Check for file watcher events
    pub fn check_file_events(&mut self) -> Option<Vec<WatchEvent>> {
        self.file_watcher.as_ref()?.try_recv()
    }

    /// Cycle strictness level
    pub fn cycle_strictness(&mut self) {
        let new_strictness = self.analyzer.config().strictness.cycle();
        self.analyzer.config_mut().strictness = new_strictness;
        self.set_status(format!("Strictness: {}", new_strictness.as_str()));
        self.scan(); // Rescan with new strictness
    }

    /// Cycle finding filter
    pub fn cycle_filter(&mut self) {
        self.filter = self.filter.cycle();
        self.apply_filter();
        self.set_status(format!("Filter: {}", self.filter.display_name()));
    }

    /// Set status message
    pub fn set_status(&mut self, message: String) {
        self.status_message = Some(message);
    }

    /// Get current strictness
    pub fn strictness(&self) -> Strictness {
        self.analyzer.config().strictness
    }
}
