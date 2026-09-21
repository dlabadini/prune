//! Terminal User Interface for Prune

mod app;
mod ui;

pub use app::{App, AppState};
pub use ui::draw;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use prune_core::watcher::WatchEvent;
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
            return Err(io::Error::other(e.to_string()));
        }
    };

    app.start_scan();

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

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> io::Result<()> {
    loop {
        app.poll_scan();
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
    if app.scanning() {
        match key {
            KeyCode::Char('q') => app.should_quit = true,
            KeyCode::Esc => app.set_status("This pass runs to completion".to_string()),
            _ => {}
        }
        return;
    }

    let typing = !app.query.is_empty();
    match key {
        KeyCode::Char('q') if !typing => app.should_quit = true,
        KeyCode::Char('?') if !typing => app.state = AppState::Help,
        KeyCode::Up => app.previous_finding(),
        KeyCode::Down => app.next_finding(),
        KeyCode::Char('k') if !typing => app.previous_finding(),
        KeyCode::Char('j') if !typing => app.next_finding(),
        KeyCode::Home | KeyCode::Char('g') if !typing => app.first_finding(),
        KeyCode::Char('G') if !typing => app.last_finding(),
        KeyCode::End if !typing => app.last_finding(),
        KeyCode::Enter if app.query.starts_with('/') => app.run_command(),
        KeyCode::Enter => app.enter_detail(),
        KeyCode::Char('c') if !typing => app.copy_next_step(),
        KeyCode::Char('x') if !typing => app.skip_selected(),
        KeyCode::Esc => app.clear_query(),
        KeyCode::Backspace => app.pop_query(),
        KeyCode::Char(ch) => app.push_query(ch),
        _ => {}
    }
}

fn handle_detail_keys(app: &mut App, key: KeyCode) {
    match key {
        KeyCode::Char('q') | KeyCode::Esc | KeyCode::Backspace => app.exit_detail(),
        KeyCode::Char('?') => app.state = AppState::Help,
        KeyCode::Up | KeyCode::Char('k') => app.scroll_detail_up(),
        KeyCode::Down | KeyCode::Char('j') => app.scroll_detail_down(),
        KeyCode::Char('c') => app.copy_next_step(),
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
