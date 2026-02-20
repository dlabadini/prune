//! TUI rendering

use super::app::{App, AppState};
use prune_core::patterns::Severity;
use prune_core::plan::RefactorPlan;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Clear, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Wrap,
    },
    Frame,
};

/// Main draw function
pub fn draw(f: &mut Frame, app: &App) {
    match app.state {
        AppState::List => draw_list_view(f, app),
        AppState::Detail => draw_detail_view(f, app),
        AppState::Help => draw_help_overlay(f, app),
    }
}

fn draw_list_view(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(0),    // Findings list
            Constraint::Length(3), // Status bar
        ])
        .split(f.area());

    draw_header(f, app, chunks[0]);
    draw_findings_list(f, app, chunks[1]);
    draw_status_bar(f, app, chunks[2]);
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let stats = app
        .last_scan
        .as_ref()
        .map(|s| {
            format!(
                "{} files | {} symbols | {} issues",
                s.files_scanned,
                s.symbols_found,
                app.findings.len()
            )
        })
        .unwrap_or_else(|| "Not scanned".to_string());

    let header_text = vec![Line::from(vec![
        Span::styled(
            " Prune ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(stats, Style::default().fg(Color::Gray)),
    ])];

    let header = Paragraph::new(header_text).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    f.render_widget(header, area);
}

fn draw_findings_list(f: &mut Frame, app: &App, area: Rect) {
    if app.filtered_findings.is_empty() {
        let message = if app.findings.is_empty() {
            "No issues found! Your codebase looks clean."
        } else {
            "No issues match the current filter."
        };

        let paragraph = Paragraph::new(message)
            .style(Style::default().fg(Color::Green))
            .block(Block::default().borders(Borders::ALL).title(" Findings "));

        f.render_widget(paragraph, area);
        return;
    }

    let items: Vec<ListItem> = app
        .filtered_findings
        .iter()
        .enumerate()
        .map(|(i, &idx)| {
            let finding = &app.findings[idx];
            let is_selected = i == app.selected_index;

            let severity_color = severity_color(finding.severity);
            let severity_indicator = match finding.severity {
                Severity::Critical => "!!!!",
                Severity::High => " !! ",
                Severity::Medium => " !  ",
                Severity::Low => " .  ",
            };

            let line = Line::from(vec![
                Span::styled(severity_indicator, Style::default().fg(severity_color)),
                Span::raw(" "),
                Span::styled(
                    finding.pattern_type.display_name(),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw(": "),
                Span::styled(
                    &finding.title,
                    if is_selected {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
            ]);

            if is_selected {
                ListItem::new(line).style(Style::default().bg(Color::DarkGray))
            } else {
                ListItem::new(line)
            }
        })
        .collect();

    let title = format!(
        " Findings ({}/{}) [{}] ",
        app.filtered_findings.len(),
        app.findings.len(),
        app.filter.display_name()
    );

    let list = List::new(items).block(Block::default().borders(Borders::ALL).title(title));

    f.render_widget(list, area);
}

fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let watch_status = if app.watch_state.is_watching {
        Span::styled(
            " WATCHING ",
            Style::default().fg(Color::Black).bg(Color::Green),
        )
    } else {
        Span::styled(" STATIC ", Style::default().fg(Color::Gray))
    };

    let strictness = Span::styled(
        format!(" {} ", app.strictness().as_str().to_uppercase()),
        Style::default().fg(Color::Cyan),
    );

    let shortcuts = Span::styled(
        " j/k:nav  Enter:detail  c:copy  p:save  w:watch  s:strict  f:filter  r:rescan  ?:help  q:quit ",
        Style::default().fg(Color::DarkGray),
    );

    let status = app.status_message.as_deref().unwrap_or("");
    let status_span = Span::styled(status, Style::default().fg(Color::Yellow));

    let line = Line::from(vec![watch_status, strictness, Span::raw(" "), status_span]);

    let status_bar = Paragraph::new(vec![line, Line::from(shortcuts)]).block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    f.render_widget(status_bar, area);
}

fn draw_detail_view(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Header
            Constraint::Min(0),    // Detail content
            Constraint::Length(2), // Footer
        ])
        .split(f.area());

    // Header
    if let Some(finding) = app.selected_finding() {
        let header = Paragraph::new(Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled(
                finding.pattern_type.display_name(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(": "),
            Span::styled(
                &finding.title,
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]))
        .block(Block::default().borders(Borders::BOTTOM));

        f.render_widget(header, chunks[0]);

        // Generate and display plan
        let plan = RefactorPlan::generate(finding);
        let plan_text = plan.as_markdown();

        // Split into lines and apply scroll
        let lines: Vec<Line> = plan_text
            .lines()
            .skip(app.detail_scroll)
            .map(|line: &str| {
                // Syntax highlighting for markdown
                if line.starts_with("# ") {
                    Line::from(Span::styled(
                        line,
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ))
                } else if line.starts_with("## ") {
                    Line::from(Span::styled(
                        line,
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ))
                } else if line.starts_with("**") && line.ends_with("**") {
                    Line::from(Span::styled(
                        line,
                        Style::default().add_modifier(Modifier::BOLD),
                    ))
                } else if line.starts_with("- [ ]") {
                    Line::from(Span::styled(line, Style::default().fg(Color::Magenta)))
                } else if line.starts_with("```") {
                    Line::from(Span::styled(line, Style::default().fg(Color::DarkGray)))
                } else if line.starts_with("|") {
                    Line::from(Span::styled(line, Style::default().fg(Color::Gray)))
                } else {
                    Line::from(line)
                }
            })
            .collect();

        let total_lines = plan_text.lines().count();
        let visible_lines = chunks[1].height as usize;

        let content = Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Refactor Plan "),
            )
            .wrap(Wrap { trim: false });

        f.render_widget(content, chunks[1]);

        // Scrollbar
        if total_lines > visible_lines {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
            let mut scrollbar_state = ScrollbarState::new(total_lines).position(app.detail_scroll);
            f.render_stateful_widget(scrollbar, chunks[1], &mut scrollbar_state);
        }
    }

    // Footer
    let footer = Paragraph::new(Line::from(vec![Span::styled(
        " Esc:back  j/k:scroll  c:copy  p:save ",
        Style::default().fg(Color::DarkGray),
    )]))
    .block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    f.render_widget(footer, chunks[2]);
}

fn draw_help_overlay(f: &mut Frame, app: &App) {
    // Draw the list view behind
    draw_list_view(f, app);

    // Calculate centered overlay
    let area = f.area();
    let width = 60.min(area.width.saturating_sub(4));
    let height = 20.min(area.height.saturating_sub(4));
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 2;
    let overlay_area = Rect::new(x, y, width, height);

    // Clear the overlay area
    f.render_widget(Clear, overlay_area);

    let help_text = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Navigation",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  j/↓        Next finding"),
        Line::from("  k/↑        Previous finding"),
        Line::from("  g/Home     First finding"),
        Line::from("  G/End      Last finding"),
        Line::from("  Enter      View detail"),
        Line::from(""),
        Line::from(Span::styled(
            "  Actions",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  c          Copy plan to clipboard"),
        Line::from("  p          Save plan to REFACTOR_PLAN.md"),
        Line::from("  w          Toggle watch mode"),
        Line::from("  s          Cycle strictness level"),
        Line::from("  f          Cycle filter by pattern"),
        Line::from("  r          Rescan codebase"),
        Line::from(""),
        Line::from(Span::styled(
            "  General",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  ?          Toggle this help"),
        Line::from("  q/Esc      Quit / Back"),
    ];

    let help = Paragraph::new(help_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Help "),
        )
        .style(Style::default().bg(Color::Black));

    f.render_widget(help, overlay_area);
}

fn severity_color(severity: Severity) -> Color {
    match severity {
        Severity::Critical => Color::Magenta,
        Severity::High => Color::Red,
        Severity::Medium => Color::Yellow,
        Severity::Low => Color::Blue,
    }
}
