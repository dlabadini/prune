//! File watching for live monitoring

use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{new_debouncer, DebouncedEvent, Debouncer};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WatcherError {
    #[error("Failed to create watcher: {0}")]
    CreateError(#[from] notify::Error),
    #[error("Failed to watch path: {0}")]
    WatchError(String),
}

/// Events from the file watcher
#[derive(Debug, Clone)]
pub enum WatchEvent {
    /// A file was modified
    Modified(PathBuf),
    /// A file was created
    Created(PathBuf),
    /// A file was deleted
    Deleted(PathBuf),
    /// An error occurred
    Error(String),
}

/// File watcher for monitoring codebase changes
pub struct FileWatcher {
    debouncer: Debouncer<RecommendedWatcher>,
    receiver: Receiver<Result<Vec<DebouncedEvent>, notify::Error>>,
    root: PathBuf,
}

impl FileWatcher {
    /// Create a new file watcher for a directory
    pub fn new(root: &Path, debounce_ms: u64) -> Result<Self, WatcherError> {
        let (tx, rx) = channel();

        let debouncer = new_debouncer(Duration::from_millis(debounce_ms), tx)?;

        Ok(Self {
            debouncer,
            receiver: rx,
            root: root.to_path_buf(),
        })
    }

    /// Start watching the root directory
    pub fn start(&mut self) -> Result<(), WatcherError> {
        self.debouncer
            .watcher()
            .watch(&self.root, RecursiveMode::Recursive)
            .map_err(|e| WatcherError::WatchError(e.to_string()))?;
        Ok(())
    }

    /// Stop watching
    pub fn stop(&mut self) -> Result<(), WatcherError> {
        self.debouncer
            .watcher()
            .unwatch(&self.root)
            .map_err(|e| WatcherError::WatchError(e.to_string()))?;
        Ok(())
    }

    /// Try to receive events (non-blocking)
    pub fn try_recv(&self) -> Option<Vec<WatchEvent>> {
        match self.receiver.try_recv() {
            Ok(Ok(events)) => Some(self.process_events(events)),
            Ok(Err(e)) => Some(vec![WatchEvent::Error(e.to_string())]),
            Err(_) => None,
        }
    }

    /// Receive events (blocking with timeout)
    pub fn recv_timeout(&self, timeout: Duration) -> Option<Vec<WatchEvent>> {
        match self.receiver.recv_timeout(timeout) {
            Ok(Ok(events)) => Some(self.process_events(events)),
            Ok(Err(e)) => Some(vec![WatchEvent::Error(e.to_string())]),
            Err(_) => None,
        }
    }

    /// Process raw events into WatchEvents
    fn process_events(&self, events: Vec<DebouncedEvent>) -> Vec<WatchEvent> {
        events
            .into_iter()
            .filter_map(|event| {
                let path = event.path;

                // Filter to only source files
                if !is_source_file(&path) {
                    return None;
                }

                // Skip files in ignored directories
                if is_ignored_path(&path) {
                    return None;
                }

                Some(if path.exists() {
                    if path.metadata().ok()?.created().ok()?
                        > std::time::SystemTime::now() - Duration::from_secs(1)
                    {
                        WatchEvent::Created(path)
                    } else {
                        WatchEvent::Modified(path)
                    }
                } else {
                    WatchEvent::Deleted(path)
                })
            })
            .collect()
    }
}

fn is_source_file(path: &Path) -> bool {
    if !path.is_file() && path.exists() {
        return false;
    }

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    matches!(
        ext,
        "ts" | "tsx" | "js" | "jsx" | "mts" | "mjs" | "cts" | "cjs"
    )
}

fn is_ignored_path(path: &Path) -> bool {
    let path_str = path.to_string_lossy();

    path_str.contains("node_modules")
        || path_str.contains(".git")
        || path_str.contains(".prune")
        || path_str.contains("dist")
        || path_str.contains("build")
        || path_str.contains(".next")
        || path_str.contains(".turbo")
}

/// Watch mode state for the TUI
#[derive(Debug, Clone, Default)]
pub struct WatchState {
    pub is_watching: bool,
    pub files_changed: Vec<PathBuf>,
    pub last_event_time: Option<std::time::Instant>,
}

impl WatchState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&mut self) {
        self.is_watching = true;
        self.files_changed.clear();
    }

    pub fn stop(&mut self) {
        self.is_watching = false;
    }

    pub fn add_changed_file(&mut self, path: PathBuf) {
        if !self.files_changed.contains(&path) {
            self.files_changed.push(path);
        }
        self.last_event_time = Some(std::time::Instant::now());
    }

    pub fn clear_changes(&mut self) {
        self.files_changed.clear();
    }

    pub fn time_since_last_event(&self) -> Option<Duration> {
        self.last_event_time.map(|t| t.elapsed())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_source_file() {
        assert!(is_source_file(Path::new("foo.ts")));
        assert!(is_source_file(Path::new("bar.tsx")));
        assert!(is_source_file(Path::new("baz.js")));
        assert!(!is_source_file(Path::new("qux.rs")));
        assert!(!is_source_file(Path::new("file.json")));
    }

    #[test]
    fn test_is_ignored_path() {
        assert!(is_ignored_path(Path::new("node_modules/foo.js")));
        assert!(is_ignored_path(Path::new(".git/config")));
        assert!(!is_ignored_path(Path::new("src/app.ts")));
    }

    #[test]
    fn test_watch_state() {
        let mut state = WatchState::new();
        assert!(!state.is_watching);

        state.start();
        assert!(state.is_watching);

        state.add_changed_file(PathBuf::from("foo.ts"));
        assert_eq!(state.files_changed.len(), 1);

        state.add_changed_file(PathBuf::from("foo.ts")); // Duplicate
        assert_eq!(state.files_changed.len(), 1);

        state.clear_changes();
        assert!(state.files_changed.is_empty());
    }
}
