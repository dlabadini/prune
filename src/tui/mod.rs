//! Terminal User Interface for Prune

mod app;
mod ui;

pub use app::{App, AppState};
pub use ui::draw;

use prune_core::watcher::WatchEvent;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::path::Path;
use std::time::Duration;

/// Run the interactive TUI
pub fn run(root: &Path) -> io::Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app
    let mut app = match App::new(root) {
        Ok(app) => app,
        Err(e) => {
            // Restore terminal before panicking
            disable_raw_mode()?;
            execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
            return Err(io::Error::new(io::ErrorKind::Other, e.to_string()));
        }
    };

    // Initial scan
    app.scan();

    // Main loop
    let result = run_app(&mut terminal, &mut app);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    loop {
        terminal.draw(|f| draw(f, app))?;

        // Handle file watcher events if in watch mode
        if app.watch_state.is_watching {
            if let Some(events) = app.check_file_events() {
                for event in events {
                    match event {
                        WatchEvent::Modified(path) | WatchEvent::Created(path) => {
                            app.watch_state.add_changed_file(path.clone());
                            app.reindex_file(&path);
                        }
                        WatchEvent::Deleted(path) => {
                            app.watch_state.add_changed_file(path.clone());
                            app.reindex_file(&path);
                        }
                        WatchEvent::Error(e) => {
                            app.set_status(format!("Watch error: {}", e));
                        }
                    }
                }

                // Re-run pattern detection after file changes
                if !app.watch_state.files_changed.is_empty() {
                    app.refresh_findings();
                }
            }
        }

        // Poll for keyboard events
        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                // Handle Ctrl+C
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    return Ok(());
                }

                match app.state {
                    AppState::List => handle_list_keys(app, key.code),
                    AppState::Detail => handle_detail_keys(app, key.code),
                    AppState::Help => handle_help_keys(app, key.code),
                }

                if app.should_quit {
                    return Ok(());
                }
            }
        }
    }
}

fn handle_list_keys(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Char('?') => app.state = AppState::Help,
        KeyCode::Up | KeyCode::Char('k') => app.previous_finding(),
        KeyCode::Down | KeyCode::Char('j') => app.next_finding(),
        KeyCode::Enter => app.enter_detail(),
        KeyCode::Char('c') => app.copy_plan_to_clipboard(),
        KeyCode::Char('p') => app.save_plan_to_file(),
        KeyCode::Char('w') => app.toggle_watch_mode(),
        KeyCode::Char('s') => app.cycle_strictness(),
        KeyCode::Char('f') => app.cycle_filter(),
        KeyCode::Char('r') => app.scan(),
        KeyCode::Home | KeyCode::Char('g') => app.first_finding(),
        KeyCode::End | KeyCode::Char('G') => app.last_finding(),
        _ => {}
    }
}

fn handle_detail_keys(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Char('q') | KeyCode::Esc | KeyCode::Backspace => app.exit_detail(),
        KeyCode::Char('?') => app.state = AppState::Help,
        KeyCode::Up | KeyCode::Char('k') => app.scroll_detail_up(),
        KeyCode::Down | KeyCode::Char('j') => app.scroll_detail_down(),
        KeyCode::Char('c') => app.copy_plan_to_clipboard(),
        KeyCode::Char('p') => app.save_plan_to_file(),
        KeyCode::PageUp => app.scroll_detail_page_up(),
        KeyCode::PageDown => app.scroll_detail_page_down(),
        _ => {}
    }
}

fn handle_help_keys(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Char('q') | KeyCode::Esc | KeyCode::Char('?') => app.state = AppState::List,
        _ => {}
    }
}
