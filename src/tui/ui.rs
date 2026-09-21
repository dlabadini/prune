//! TUI rendering

use super::app::{App, AppState};
use prune_core::patterns::Tier;
use prune_core::plan::RefactorPlan;
use prune_core::score::{self, Outcome};
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
            Constraint::Length(3),
            Constraint::Min(4),
            Constraint::Length(4),
            Constraint::Length(3),
        ])
        .split(f.area());

    draw_header(f, app, chunks[0]);
    draw_findings_list(f, app, chunks[1]);
    draw_strip(f, app, chunks[2]);
    draw_status_bar(f, app, chunks[3]);
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let root = app
        .root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(".");
    let (title, pools) = if let Some(score) = &app.score {
        (
            format!(" {}  {}", score.summary(), root),
            format!(
                " Essential {:.0}/{:.0}   Recommended {:.0}/{:.0}   Bonus {:.0}",
                score.essential_earned,
                score.essential_available,
                score.recommended_earned,
                score.recommended_available,
                score.bonus
            ),
        )
    } else {
        (" Scanning".to_string(), String::new())
    };

    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                " Prune",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(title, Style::default().fg(Color::White)),
        ]),
        Line::from(Span::styled(pools, Style::default().fg(Color::Gray))),
    ])
    .block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(Color::DarkGray)),
    );

    f.render_widget(header, area);
}

fn draw_findings_list(f: &mut Frame, app: &App, area: Rect) {
    if app.filtered_findings.is_empty() {
        let message = if app.scanning() {
            "Scanning..."
        } else if app.findings.is_empty() {
            "No issues found."
        } else if !app.query.is_empty() && !app.query.starts_with('/') {
            "No issues match this text."
        } else {
            "Nothing left in this session."
        };
        let paragraph = Paragraph::new(message).style(Style::default().fg(Color::Gray));
        f.render_widget(paragraph, area);
        return;
    }

    let visible_rows = (area.height as usize / 2).max(1);
    let start = app
        .selected_index
        .saturating_add(1)
        .saturating_sub(visible_rows)
        .min(app.filtered_findings.len().saturating_sub(visible_rows));

    let items: Vec<ListItem> = app
        .filtered_findings
        .iter()
        .enumerate()
        .skip(start)
        .take(visible_rows)
        .map(|(index, &finding_index)| {
            let finding = &app.findings[finding_index];
            let selected = index == app.selected_index;
            let marker = if selected { ">" } else { " " };
            let outcome = score::finding_outcome(finding);
            let outcome_color = match outcome {
                Outcome::Fail => Color::Red,
                Outcome::Partial => Color::Yellow,
                Outcome::Pass => Color::Green,
            };
            let places = finding
                .locations
                .iter()
                .take(2)
                .map(|location| format!("{}:{}", location.file, location.start_line))
                .collect::<Vec<_>>()
                .join(" · ");
            let style = if selected {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(vec![
                Line::from(vec![
                    Span::raw(format!(" {marker} ")),
                    Span::styled(
                        format!("{:<11}", finding.pattern_type.tier().as_str()),
                        Style::default().fg(tier_color(finding.pattern_type.tier())),
                    ),
                    Span::styled(
                        format!("{:<8}", outcome.as_str()),
                        Style::default().fg(outcome_color),
                    ),
                    Span::styled(finding.pattern_type.display_name(), style.fg(Color::White)),
                ]),
                Line::from(Span::styled(
                    format!(
                        "    {}",
                        if places.is_empty() {
                            finding.title.clone()
                        } else {
                            places
                        }
                    ),
                    Style::default().fg(Color::DarkGray),
                )),
            ])
        })
        .collect();

    f.render_widget(List::new(items), area);
}

fn draw_strip(f: &mut Frame, app: &App, area: Rect) {
    let lines = if app.query.starts_with('/') {
        let mut lines = vec![Line::from(Span::styled(
            format!(" > {}", app.query),
            Style::default().fg(Color::Cyan),
        ))];
        for (name, help) in app.matching_commands().into_iter().take(2) {
            lines.push(Line::from(Span::styled(
                format!("   /{name}  {help}"),
                Style::default().fg(Color::Gray),
            )));
        }
        lines
    } else if let Some(finding) = app.selected_finding() {
        vec![
            Line::from(Span::styled(
                truncate(&score::evidence_line(finding), area.width as usize),
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                truncate(
                    &format!(" Next: {}", score::next_line(finding)),
                    area.width as usize,
                ),
                Style::default().fg(Color::Yellow),
            )),
            Line::from(Span::styled(
                " c copy this    enter full plan    x skip",
                Style::default().fg(Color::DarkGray),
            )),
        ]
    } else {
        vec![Line::from(Span::styled(
            " c copy this    enter full plan",
            Style::default().fg(Color::DarkGray),
        ))]
    };

    let paragraph = Paragraph::new(lines).block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(paragraph, area);
}

fn draw_status_bar(f: &mut Frame, app: &App, area: Rect) {
    let query_line = if app.query.is_empty() {
        Line::from(Span::styled(
            " type to filter    / commands    ? help",
            Style::default().fg(Color::DarkGray),
        ))
    } else if app.query.starts_with('/') {
        Line::from(Span::styled(
            " enter runs    esc clears",
            Style::default().fg(Color::DarkGray),
        ))
    } else {
        Line::from(vec![
            Span::styled(
                format!(" > {}", app.query),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                format!("    {} matches", app.filtered_findings.len()),
                Style::default().fg(Color::DarkGray),
            ),
        ])
    };
    let status = app.status_message.as_deref().unwrap_or("");
    let status_bar = Paragraph::new(vec![
        query_line,
        Line::from(Span::styled(status, Style::default().fg(Color::Yellow))),
    ]);
    f.render_widget(status_bar, area);
}

fn draw_detail_view(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(f.area());

    if let Some(finding) = app.selected_finding() {
        let header = Paragraph::new(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                finding.pattern_type.display_name(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(finding.id.clone(), Style::default().fg(Color::DarkGray)),
        ]))
        .block(Block::default().borders(Borders::BOTTOM));
        f.render_widget(header, chunks[0]);

        let plan = RefactorPlan::generate(finding);
        let plan_text = plan.as_markdown();
        let lines: Vec<Line> = plan_text
            .lines()
            .skip(app.detail_scroll)
            .map(|line: &str| {
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
                } else {
                    Line::from(line)
                }
            })
            .collect();
        let total_lines = plan_text.lines().count();
        let content = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" Plan "))
            .wrap(Wrap { trim: false });
        f.render_widget(content, chunks[1]);

        if total_lines > chunks[1].height as usize {
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
            let mut scrollbar_state = ScrollbarState::new(total_lines).position(app.detail_scroll);
            f.render_stateful_widget(scrollbar, chunks[1], &mut scrollbar_state);
        }
    }

    let footer = Paragraph::new(Line::from(Span::styled(
        " esc back    j/k scroll    c copy this",
        Style::default().fg(Color::DarkGray),
    )))
    .block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(footer, chunks[2]);
}

fn draw_help_overlay(f: &mut Frame, app: &App) {
    draw_list_view(f, app);

    let area = f.area();
    let width = 64.min(area.width.saturating_sub(4));
    let height = 22.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let overlay_area = Rect::new(x, y, width, height);
    f.render_widget(Clear, overlay_area);

    let help_text = vec![
        Line::from(""),
        Line::from(Span::styled(
            "  List",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  j/k or arrows   Move"),
        Line::from("  enter           Full plan"),
        Line::from("  c               Copy the next step"),
        Line::from("  x               Skip until the next scan"),
        Line::from("  letters         Filter titles and paths"),
        Line::from("  /               Commands, then enter"),
        Line::from("  esc             Clear the line, or stop waiting"),
        Line::from(""),
        Line::from(Span::styled(
            "  Commands",
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from("  /rescan  /strict  /watch  /sort  /save"),
        Line::from(""),
        Line::from("  The score ignores a session skip."),
        Line::from("  A finding leaves the score when a rescan"),
        Line::from("  no longer reports that id."),
        Line::from(""),
        Line::from("  q quit    ? close"),
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

fn tier_color(tier: Tier) -> Color {
    match tier {
        Tier::Essential => Color::Red,
        Tier::Recommended => Color::Yellow,
        Tier::Emerging => Color::Blue,
    }
}

fn truncate(text: &str, width: usize) -> String {
    let width = width.saturating_sub(1);
    let mut out = String::new();
    for ch in text.chars() {
        if out.chars().count() + 1 > width {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}
