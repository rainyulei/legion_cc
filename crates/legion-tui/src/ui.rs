//! TUI rendering - header + bordered PTY area + footer + popup overlay

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};
use tui_term::widget::PseudoTerminal;

use legion_core::orchestrate::WorkerTaskStatus;

use crate::app::{App, AppMode, MainMenuItem, MatrixCol, ModelTarget, PopupMenu};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Main draw function
pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header
            Constraint::Min(0),    // Main content (PTY)
            Constraint::Length(1), // Footer
        ])
        .split(frame.area());

    draw_header(frame, app, chunks[0]);
    draw_main(frame, app, chunks[1]);
    draw_footer(frame, app, chunks[2]);

    // Draw popup overlay if in popup mode
    if let AppMode::Popup(menu) = app.mode {
        draw_popup(frame, app, menu);
    }

    // Draw dashboard overlay if toggled
    if app.show_dashboard {
        draw_dashboard_overlay(frame, app);
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let focused_pane = app.panes.get(app.focused_pane);
    let provider_name = focused_pane
        .and_then(|p| p.current_provider)
        .and_then(|i| app.providers.get(i))
        .map(|p| p.name.as_str())
        .unwrap_or("No Provider");
    let model_name = focused_pane
        .and_then(|p| p.current_model.as_deref())
        .unwrap_or("No Model");

    let indicator = if app.provider_connected {
        Span::styled(" \u{25cf}", Style::default().fg(Color::Green))
    } else {
        Span::styled(" \u{25cb}", Style::default().fg(Color::DarkGray))
    };

    let header = Line::from(vec![
        Span::styled(
            format!(" Legion v{}", VERSION),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("        "),
        Span::styled("[", Style::default().fg(Color::DarkGray)),
        Span::styled(provider_name, Style::default().fg(Color::Yellow)),
        Span::styled(" \u{2192} ", Style::default().fg(Color::DarkGray)),
        Span::styled(model_name, Style::default().fg(Color::Magenta)),
        Span::styled("]", Style::default().fg(Color::DarkGray)),
        indicator,
    ]);

    frame.render_widget(Paragraph::new(header), area);
}

fn draw_main(frame: &mut Frame, app: &App, area: Rect) {
    if app.is_squad() {
        draw_squad_layout(frame, app, area);
    } else {
        draw_pane(frame, app, 0, area);
    }
}

/// Squad mode: leader | divider | workers, vertically split on right
fn draw_squad_layout(frame: &mut Frame, app: &App, area: Rect) {
    let leader_width = (area.width as u32 * app.leader_ratio as u32 / 100) as u16;
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(leader_width),
            Constraint::Length(1), // divider column
            Constraint::Min(0),   // workers get the rest
        ])
        .split(area);

    // Left: Leader pane (index 0)
    draw_pane(frame, app, 0, h_chunks[0]);

    // Center: draggable divider
    draw_divider(frame, app, h_chunks[1]);

    // Right: Worker panes vertically split
    let worker_count = app.panes.len() - 1;
    if worker_count > 0 {
        let constraints: Vec<Constraint> = (0..worker_count)
            .map(|_| Constraint::Ratio(1, worker_count as u32))
            .collect();
        let v_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
            .split(h_chunks[2]);

        for i in 0..worker_count {
            draw_pane(frame, app, i + 1, v_chunks[i]);
        }
    }
}

/// Draw the vertical divider between leader and workers
fn draw_divider(frame: &mut Frame, app: &App, area: Rect) {
    let (ch, style) = if app.dragging_divider {
        ("\u{2503}", Style::default().fg(Color::Yellow)) // ┃ thick, yellow
    } else if app.hover_on_divider {
        ("\u{2502}", Style::default().fg(Color::Cyan))   // │ cyan
    } else {
        ("\u{2502}", Style::default().fg(Color::DarkGray)) // │ dim
    };

    let lines: Vec<Line> = (0..area.height)
        .map(|_| Line::from(Span::styled(ch, style)))
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// Render a single pane with border and PTY content
fn draw_pane(frame: &mut Frame, app: &App, index: usize, area: Rect) {
    let pane = match app.panes.get(index) {
        Some(p) => p,
        None => return,
    };

    let is_focused = app.focused_pane == index;
    let border_color = if is_focused { Color::Blue } else { Color::DarkGray };

    let title = if app.is_squad() {
        let model = pane.current_model.as_deref().unwrap_or("--");
        if let Some(ws) = app.worker_task_status(index) {
            let icon = match ws.status {
                WorkerTaskStatus::Working => ">>",
                WorkerTaskStatus::Done => "OK",
                WorkerTaskStatus::Error => "ERR",
                WorkerTaskStatus::Pending => "??",
                WorkerTaskStatus::Stopped => "XX",
                WorkerTaskStatus::Idle => "--",
            };
            let ticket_short = ws.ticket.as_deref()
                .map(|t| if t.len() > 20 { format!("{}...", &t[..17]) } else { t.to_string() })
                .unwrap_or_default();
            let elapsed = format_elapsed(ws.elapsed_secs);
            format!(" {} | {} [{}] {} [{}] ", pane.label, model, icon, ticket_short, elapsed)
        } else {
            format!(" {} | {} ", pane.label, model)
        }
    } else {
        " Claude Code ".to_string()
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    if let Some(parser) = app.parser_at(index) {
        if let Ok(p) = parser.lock() {
            let pseudo_term = PseudoTerminal::new(p.screen()).block(block);
            frame.render_widget(pseudo_term, area);
            return;
        }
    }

    // Fallback: no PTY running yet
    let content = Paragraph::new(" Starting Claude Code...")
        .style(Style::default().fg(Color::DarkGray))
        .block(block);
    frame.render_widget(content, area);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let mode_hint = if app.is_squad() && app.mode == AppMode::Normal {
        let progress = app.orchestrate_snapshot.as_ref().map(|snap| {
            let total = snap.len();
            let done = snap.iter().filter(|w| w.status == WorkerTaskStatus::Done).count();
            format!("{}/{}", done, total)
        }).unwrap_or_default();
        vec![
            Span::styled(format!(" Workers: {}", progress), Style::default().fg(Color::Cyan)),
            Span::styled(" \u{2502} ", Style::default().fg(Color::DarkGray)),
            Span::styled("Tab", Style::default().fg(Color::Yellow)),
            Span::styled(": Focus ", Style::default().fg(Color::DarkGray)),
            Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
            Span::styled("Ctrl+T", Style::default().fg(Color::Yellow)),
            Span::styled(": Dashboard ", Style::default().fg(Color::DarkGray)),
            Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
            Span::styled("Ctrl+P", Style::default().fg(Color::Yellow)),
            Span::styled(": Menu ", Style::default().fg(Color::DarkGray)),
            Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
            Span::styled("Ctrl+Q", Style::default().fg(Color::Yellow)),
            Span::styled(": Quit", Style::default().fg(Color::DarkGray)),
        ]
    } else {
        match app.mode {
            AppMode::Normal => vec![
                Span::styled(" Ctrl+P", Style::default().fg(Color::Yellow)),
                Span::styled(": Menu ", Style::default().fg(Color::DarkGray)),
                Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
                Span::styled("Ctrl+Q", Style::default().fg(Color::Yellow)),
                Span::styled(": Quit", Style::default().fg(Color::DarkGray)),
            ],
            AppMode::Popup(popup) => match popup {
                PopupMenu::Matrix => vec![
                    Span::styled(" Tab", Style::default().fg(Color::Yellow)),
                    Span::styled(": Column ", Style::default().fg(Color::DarkGray)),
                    Span::styled("j/k", Style::default().fg(Color::Yellow)),
                    Span::styled(": Row ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Enter", Style::default().fg(Color::Yellow)),
                    Span::styled(": Edit ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Esc", Style::default().fg(Color::Yellow)),
                    Span::styled(": Back", Style::default().fg(Color::DarkGray)),
                ],
                _ => vec![
                    Span::styled(" j/k", Style::default().fg(Color::Yellow)),
                    Span::styled(": Navigate ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Enter", Style::default().fg(Color::Yellow)),
                    Span::styled(": Select ", Style::default().fg(Color::DarkGray)),
                    Span::styled("Esc", Style::default().fg(Color::Yellow)),
                    Span::styled(": Close", Style::default().fg(Color::DarkGray)),
                ],
            },
        }
    };

    frame.render_widget(Paragraph::new(Line::from(mode_hint)), area);
}

// --- Popup overlay ---

fn draw_popup(frame: &mut Frame, app: &App, menu: PopupMenu) {
    let area = centered_rect(60, 60, frame.area());
    frame.render_widget(Clear, area);

    match menu {
        PopupMenu::Main => draw_main_menu(frame, app, area),
        PopupMenu::Matrix => draw_matrix(frame, app, area),
        PopupMenu::Provider => draw_provider_menu(frame, app, area),
        PopupMenu::Model => draw_model_menu(frame, app, area),
        PopupMenu::SessionList => draw_session_list(frame, app, area),
        PopupMenu::CompleteSession => draw_complete_session(frame, app, area),
    }
}

fn draw_main_menu(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Legion [ESC] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::DarkGray));

    let items: Vec<ListItem> = App::main_menu_items()
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let selected = i == app.menu_index;
            let prefix = if selected { "> " } else { "  " };
            let value = match item {
                MainMenuItem::SwitchModels => {
                    let n = app.panes.len();
                    if n == 0 { "[no panes]".to_string() }
                    else { format!("[{} pane{}]", n, if n == 1 { "" } else { "s" }) }
                }
                MainMenuItem::SwitchSession => {
                    format!("[{}]", app.session_name())
                }
                MainMenuItem::CompleteSession => String::new(),
                MainMenuItem::Quit => String::new(),
            };

            let style = if selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            if value.is_empty() {
                ListItem::new(Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(item.label(), style),
                ]))
            } else {
                let pad = " ".repeat(20usize.saturating_sub(prefix.len() + item.label().len()));
                ListItem::new(Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(item.label(), style),
                    Span::raw(pad),
                    Span::styled(value, Style::default().fg(Color::DarkGray)),
                ]))
            }
        })
        .collect();

    // Separator before the last item (Quit)
    let quit_idx = items.len() - 1;
    let mut final_items = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        if i == quit_idx {
            final_items.push(ListItem::new(Line::from(Span::styled(
                "  \u{2500}".repeat(12),
                Style::default().fg(Color::DarkGray),
            ))));
        }
        final_items.push(item);
    }

    frame.render_widget(List::new(final_items).block(block), area);
}

fn draw_matrix(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Switch Model [ESC] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::DarkGray));

    let pane_count = app.panes.len();
    let mut items: Vec<ListItem> = Vec::new();

    // Column header
    items.push(ListItem::new(Line::from(vec![
        Span::styled("  Pane            ", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        Span::styled("Provider          ", Style::default().fg(
            if app.matrix_col == MatrixCol::Provider { Color::Yellow } else { Color::DarkGray }
        ).add_modifier(Modifier::BOLD)),
        Span::styled("Model", Style::default().fg(
            if app.matrix_col == MatrixCol::Model { Color::Magenta } else { Color::DarkGray }
        ).add_modifier(Modifier::BOLD)),
    ])));

    // Pane rows
    for (i, pane) in app.panes.iter().enumerate() {
        let is_row = app.matrix_row == i;
        items.push(matrix_row_item(app, is_row, &pane.label, pane.current_provider, pane.current_model.as_deref()));
    }

    // Separator + batch rows (squad mode only)
    if app.is_squad() {
        items.push(ListItem::new(Line::from(Span::styled(
            "  \u{2500}".repeat(20),
            Style::default().fg(Color::DarkGray),
        ))));

        let aw_selected = app.matrix_row == pane_count;
        items.push(matrix_row_item(app, aw_selected, "All Workers", None, None));

        let ap_selected = app.matrix_row == pane_count + 1;
        items.push(matrix_row_item(app, ap_selected, "All Panes", None, None));
    }

    // Footer hint
    items.push(ListItem::new(Line::from(Span::raw(""))));
    items.push(ListItem::new(Line::from(vec![
        Span::styled("  Tab", Style::default().fg(Color::Yellow)),
        Span::styled(": Column  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Enter", Style::default().fg(Color::Yellow)),
        Span::styled(": Edit  ", Style::default().fg(Color::DarkGray)),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::styled(": Back", Style::default().fg(Color::DarkGray)),
    ])));

    frame.render_widget(List::new(items).block(block), area);
}

fn matrix_row_item<'a>(
    app: &App,
    is_selected_row: bool,
    label: &str,
    provider_idx: Option<usize>,
    model: Option<&str>,
) -> ListItem<'a> {
    let prefix = if is_selected_row { "> " } else { "  " };

    let provider_name = provider_idx
        .and_then(|i| app.providers.get(i))
        .map(|p| p.name.as_str())
        .unwrap_or("--");
    let model_name = model.unwrap_or("--");

    let provider_style = if is_selected_row && app.matrix_col == MatrixCol::Provider {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else if is_selected_row {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::Gray)
    };

    let model_style = if is_selected_row && app.matrix_col == MatrixCol::Model {
        Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)
    } else if is_selected_row {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::Gray)
    };

    let row_style = if is_selected_row {
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let label_padded = format!("{:<16}", label);
    let provider_padded = format!("{:<18}", format!("[{}]", provider_name));

    ListItem::new(Line::from(vec![
        Span::raw(prefix.to_string()),
        Span::styled(label_padded, row_style),
        Span::styled(provider_padded, provider_style),
        Span::styled(format!("[{}]", model_name), model_style),
    ]))
}

fn draw_provider_menu(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(format!(" Select Provider for {} [ESC] ", app.target_label()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(Color::DarkGray));

    if app.providers.is_empty() {
        frame.render_widget(
            Paragraph::new("No providers configured")
                .style(Style::default().fg(Color::DarkGray))
                .block(block),
            area,
        );
        return;
    }

    let items: Vec<ListItem> = app
        .providers
        .iter()
        .enumerate()
        .map(|(i, provider)| {
            let selected = i == app.submenu_index;
            let current = match app.model_target {
                Some(ModelTarget::Pane(pi)) => app.panes.get(pi)
                    .and_then(|p| p.current_provider) == Some(i),
                _ => app.current_provider == Some(i),
            };
            let prefix = if selected { "> " } else { "  " };
            let dot = if current { " \u{25cf}" } else { "" };
            let style = if selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(vec![
                Span::raw(prefix),
                Span::styled(&provider.name, style),
                Span::styled(dot, Style::default().fg(Color::Green)),
            ]))
        })
        .collect();

    frame.render_widget(List::new(items).block(block), area);
}

fn draw_model_menu(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(format!(" Select Model for {} [ESC] ", app.target_label()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta))
        .style(Style::default().bg(Color::DarkGray));

    if let Some(models) = app.target_provider_models() {
        let items: Vec<ListItem> = models
            .iter()
            .enumerate()
            .map(|(i, model)| {
                let selected = i == app.submenu_index;
                let current = match app.model_target {
                    Some(ModelTarget::Pane(pi)) => app.panes.get(pi)
                        .and_then(|p| p.current_model.as_deref()) == Some(model),
                    _ => app.current_model.as_deref() == Some(model),
                };
                let prefix = if selected { "> " } else { "  " };
                let dot = if current { " \u{25cf}" } else { "" };
                let style = if selected {
                    Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(model, style),
                    Span::styled(dot, Style::default().fg(Color::Green)),
                ]))
            })
            .collect();
        frame.render_widget(List::new(items).block(block), area);
    } else {
        frame.render_widget(
            Paragraph::new("No models available")
                .style(Style::default().fg(Color::DarkGray))
                .block(block),
            area,
        );
    }
}

fn draw_session_list(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Sessions [ESC] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::DarkGray));

    let mut items: Vec<ListItem> = Vec::new();
    let current_name = app.current_session.as_ref().map(|s| s.name.as_str());

    for (i, session) in app.session_list.iter().enumerate() {
        let selected = i == app.session_list_index;
        let prefix = if selected { "> " } else { "  " };
        let icon = if current_name == Some(session.name.as_str()) {
            "\u{25cf} "
        } else if session.status == "completed" {
            "\u{2713} "
        } else {
            "\u{25cb} "
        };
        let style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else if session.status == "completed" {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        };
        let pane_count = 1 + session.worker_count;
        items.push(ListItem::new(Line::from(vec![
            Span::raw(prefix.to_string()),
            Span::styled(icon, style),
            Span::styled(session.name.clone(), style),
            Span::styled(format!("  {} panes", pane_count), Style::default().fg(Color::DarkGray)),
        ])));
    }

    let new_selected = app.session_list_index >= app.session_list.len();
    let new_prefix = if new_selected { "> " } else { "  " };
    let new_style = if new_selected {
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Green)
    };
    items.push(ListItem::new(Line::from(vec![
        Span::raw(new_prefix.to_string()),
        Span::styled("[+] New Session", new_style),
    ])));

    frame.render_widget(List::new(items).block(block), area);
}

fn draw_complete_session(frame: &mut Frame, app: &App, area: Rect) {
    let session_name = app.session_name();
    let block = Block::default()
        .title(format!(" Complete '{}' [ESC] ", session_name))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(Color::DarkGray));

    let options = ["Merge to main", "Keep worktrees", "Discard changes"];
    let descriptions = [
        "Merge all pane branches into main, then clean up",
        "Mark completed but keep worktrees for manual handling",
        "Delete all worktrees and branches (destructive!)",
    ];

    let items: Vec<ListItem> = options.iter().zip(descriptions.iter()).enumerate()
        .map(|(i, (opt, desc))| {
            let selected = i == app.complete_merge_index;
            let prefix = if selected { "> " } else { "  " };
            let style = if selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(vec![
                Line::from(vec![Span::raw(prefix), Span::styled(*opt, style)]),
                Line::from(Span::styled(format!("    {}", desc), Style::default().fg(Color::DarkGray))),
            ])
        })
        .collect();

    frame.render_widget(List::new(items).block(block), area);
}

fn format_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else {
        format!("{}m {}s", secs / 60, secs % 60)
    }
}

fn draw_dashboard_overlay(frame: &mut Frame, app: &App) {
    let area = centered_rect(70, 50, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Squad Progress [Ctrl+T: close] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::DarkGray));

    let mut items: Vec<ListItem> = Vec::new();

    // Header
    items.push(ListItem::new(Line::from(vec![
        Span::styled("  #   ", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        Span::styled("Status  ", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        Span::styled("Task                              ", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        Span::styled("Worker    ", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        Span::styled("Time", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
    ])));

    if let Some(ref snapshot) = app.orchestrate_snapshot {
        let mut total = 0usize;
        let mut done = 0usize;
        for (i, ws) in snapshot.iter().enumerate() {
            total += 1;
            let (icon, color) = match ws.status {
                WorkerTaskStatus::Done => { done += 1; ("OK ", Color::Green) }
                WorkerTaskStatus::Error => ("ERR", Color::Red),
                WorkerTaskStatus::Working => (".. ", Color::Yellow),
                WorkerTaskStatus::Pending => (">> ", Color::Cyan),
                WorkerTaskStatus::Stopped => ("XX ", Color::DarkGray),
                WorkerTaskStatus::Idle => ("-- ", Color::DarkGray),
            };

            let ticket = ws.ticket.as_deref().unwrap_or("-");
            let ticket_display = if ticket.len() > 32 {
                format!("{}...", &ticket[..29])
            } else {
                format!("{:<32}", ticket)
            };

            let elapsed = format_elapsed(ws.elapsed_secs);

            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("  #{:<3}", i + 1), Style::default().fg(Color::White)),
                Span::styled(format!("[{}] ", icon), Style::default().fg(color)),
                Span::styled(ticket_display, Style::default().fg(Color::White)),
                Span::styled(format!("Worker {:<3}", ws.worker_id), Style::default().fg(Color::DarkGray)),
                Span::styled(elapsed, Style::default().fg(Color::DarkGray)),
            ])));
        }

        items.push(ListItem::new(Line::from(Span::raw(""))));
        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                format!("  Progress: {}/{} complete", done, total),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
        ])));
    } else {
        items.push(ListItem::new(Line::from(
            Span::styled("  No orchestration active", Style::default().fg(Color::DarkGray))
        )));
    }

    frame.render_widget(List::new(items).block(block), area);
}

/// Centered rectangle helper
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(v[1])[1]
}
