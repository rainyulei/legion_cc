//! TUI rendering - header + bordered PTY area + footer + popup overlay

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
    Frame,
};
use tui_term::widget::PseudoTerminal;

use legion_core::{TeamMode, TicketSnapshot, TicketStatus};

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

    // Draw board detail popup overlay (squad mode)
    if app.board_detail_open && app.is_squad() {
        draw_board_detail_popup(frame, app);
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

    let mut spans = vec![
        Span::styled(
            format!(" Legion v{}", VERSION),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    ];

    // Session name (squad mode only)
    if app.is_squad() {
        if let Some(ref session) = app.current_session {
            spans.push(Span::styled("  ", Style::default()));
            spans.push(Span::styled(
                format!("({})", session.name),
                Style::default().fg(Color::Green),
            ));
        }
    }

    // Queue stats (squad mode)
    if app.is_squad() {
        if let Some((total, _queued, working, done, error)) = app.queue_stats {
            spans.push(Span::styled("  ", Style::default()));
            spans.push(Span::styled(format!("W:{}", app.worker_count()), Style::default().fg(Color::Cyan)));
            spans.push(Span::styled(format!(" Q:{}", total), Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled(format!(" \u{2713}{}", done), Style::default().fg(Color::Green)));
            if working > 0 {
                spans.push(Span::styled(format!(" \u{25b6}{}", working), Style::default().fg(Color::Yellow)));
            }
            if error > 0 {
                spans.push(Span::styled(format!(" \u{2717}{}", error), Style::default().fg(Color::Red)));
            }
        }
    }

    spans.extend([
        Span::raw("  "),
        Span::styled("[", Style::default().fg(Color::DarkGray)),
        Span::styled(provider_name, Style::default().fg(Color::Yellow)),
        Span::styled(" \u{2192} ", Style::default().fg(Color::DarkGray)),
        Span::styled(model_name, Style::default().fg(Color::Magenta)),
        Span::styled("]", Style::default().fg(Color::DarkGray)),
        indicator,
    ]);

    // Git branch
    if let Some(ref branch) = app.leader_git_branch {
        spans.push(Span::styled("  ", Style::default()));
        spans.push(Span::styled(
            branch.as_str(),
            Style::default().fg(Color::Cyan),
        ));
    }

    // Context percentage with color coding
    if let Some(pct) = app.leader_context_pct {
        let ctx_color = if pct >= 80 {
            Color::Red
        } else if pct >= 50 {
            Color::Yellow
        } else {
            Color::Green
        };
        spans.push(Span::styled("  ", Style::default()));
        spans.push(Span::styled(
            format!("\u{1f9e0}{}%", pct),
            Style::default().fg(ctx_color),
        ));
    }

    let header = Line::from(spans);

    frame.render_widget(Paragraph::new(header), area);
}

fn draw_main(frame: &mut Frame, app: &App, area: Rect) {
    if app.panes.is_empty() {
        // No panes yet (startup session selection)
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));
        let content = Paragraph::new(" Select or create a session to begin...")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(content, area);
    } else if app.is_squad() {
        draw_squad_layout(frame, app, area);
    } else {
        draw_pane(frame, app, 0, area);
    }
}

/// Squad mode: leader | divider | task board
fn draw_squad_layout(frame: &mut Frame, app: &App, area: Rect) {
    let leader_width = (area.width as u32 * app.leader_ratio as u32 / 100) as u16;
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(leader_width),
            Constraint::Length(1), // divider column
            Constraint::Min(0),   // task board gets the rest
        ])
        .split(area);

    // Left: Leader PTY
    draw_pane(frame, app, 0, h_chunks[0]);

    // Center: draggable divider
    draw_divider(frame, app, h_chunks[1]);

    // Right: Squad Board (always visible; detail is a popup overlay)
    draw_task_board(frame, app, h_chunks[2]);
}

/// Draw the vertical divider between leader and task board
fn draw_divider(frame: &mut Frame, app: &App, area: Rect) {
    let (ch, style) = if app.dragging_divider {
        ("\u{2503}", Style::default().fg(Color::Yellow)) // thick, yellow
    } else if app.hover_on_divider {
        ("\u{2502}", Style::default().fg(Color::Cyan))   // cyan
    } else {
        ("\u{2502}", Style::default().fg(Color::DarkGray)) // dim
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

    let is_focused = app.focused_pane == index && !app.right_panel_focused;
    let border_color = if is_focused { Color::Blue } else { Color::DarkGray };

    let title = if app.is_squad() {
        let model = pane.current_model.as_deref().unwrap_or("--");
        format!(" {} | {} ", pane.label, model)
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

/// Draw the embedded squad board in the right panel
fn draw_task_board(frame: &mut Frame, app: &App, area: Rect) {
    let is_focused = app.right_panel_focused;
    let border_color = if is_focused { Color::Blue } else { Color::DarkGray };

    let block = Block::default()
        .title(" Squad Board ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let tickets = app.ticket_snapshot.as_deref().unwrap_or(&[]);
    if tickets.is_empty() {
        let content = Paragraph::new(" Waiting for Leader to submit tickets...")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(content, area);
        return;
    }

    // Partition tickets by status
    let working: Vec<&TicketSnapshot> = tickets.iter().filter(|t| t.status == TicketStatus::Working).collect();
    let queued: Vec<&TicketSnapshot> = tickets.iter().filter(|t| t.status == TicketStatus::Queued).collect();
    let done: Vec<&TicketSnapshot> = tickets.iter().filter(|t| t.status == TicketStatus::Done).collect();
    let errored: Vec<&TicketSnapshot> = tickets.iter().filter(|t| t.status == TicketStatus::Error).collect();

    // Build lines to render inside the outer block
    let inner = block.inner(area);
    let mut lines: Vec<Line> = Vec::new();

    // --- WORKING section: bordered cards ---
    let selected_prefix = |ticket_id: usize| -> &'static str {
        if app.right_panel_focused && app.board_selected == ticket_id { "\u{25b6} " } else { "  " }
    };

    lines.push(Line::from(Span::styled(
        format!(" \u{25b6} WORKING ({})", working.len()),
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    )));
    for t in &working {
        let w_label = t.assigned_worker.map(|w| format!("W{}", w)).unwrap_or_default();
        let team_short = team_mode_short(&t.team_mode);
        let card_width = inner.width.saturating_sub(4) as usize;
        let title_line = format!("#{} {}", t.id, t.title);
        let suffix = format!(" {} ", w_label);
        let pad_len = card_width.saturating_sub(title_line.len() + suffix.len() + 2);
        let dashes = "\u{2500}".repeat(pad_len.min(60));

        let sel = app.right_panel_focused && app.board_selected == t.id;
        let border_col = if sel { Color::Yellow } else { Color::DarkGray };

        // Top border
        let top = format!("{}\u{250c}\u{2500} {} {}{}\u{2510}",
            selected_prefix(t.id),
            title_line, dashes, suffix);
        // Truncate to fit
        let top_display: String = top.chars().take(inner.width as usize).collect();
        lines.push(Line::from(Span::styled(top_display, Style::default().fg(border_col))));

        // Content line
        let detail = format!("iter {}/{} \u{00b7} {} \u{00b7} {}",
            t.iteration, t.max_iterations, format_elapsed(t.elapsed_secs), team_short);
        let content_line = format!("  \u{2502} {} ", detail);
        let content_pad = card_width.saturating_sub(detail.len() + 2);
        let content_full = format!("{}{}\u{2502}",
            content_line, " ".repeat(content_pad.min(200)));
        let content_display: String = content_full.chars().take(inner.width as usize).collect();
        lines.push(Line::from(Span::styled(content_display, Style::default().fg(border_col))));

        // Bottom border
        let bot_inner = card_width.saturating_sub(0);
        let bot = format!("  \u{2514}{}\u{2518}", "\u{2500}".repeat(bot_inner.min(200)));
        let bot_display: String = bot.chars().take(inner.width as usize).collect();
        lines.push(Line::from(Span::styled(bot_display, Style::default().fg(border_col))));
    }

    lines.push(Line::from(Span::raw("")));

    // --- QUEUED section: compact ---
    lines.push(Line::from(Span::styled(
        format!(" \u{23f3} QUEUED ({})", queued.len()),
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    )));
    for t in &queued {
        lines.push(ticket_compact_line(t, app, false));
    }

    lines.push(Line::from(Span::raw("")));

    // --- DONE section: compact with elapsed ---
    lines.push(Line::from(Span::styled(
        format!(" \u{2713} DONE ({})", done.len()),
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
    )));
    for t in &done {
        lines.push(ticket_compact_line(t, app, true));
    }

    lines.push(Line::from(Span::raw("")));

    // --- ERROR section: compact ---
    lines.push(Line::from(Span::styled(
        format!(" \u{2717} ERROR ({})", errored.len()),
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
    )));
    for t in &errored {
        let sel = app.right_panel_focused && app.board_selected == t.id;
        let prefix = if sel { "\u{25b6} " } else { "  " };
        let row_style = if sel {
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(vec![
            Span::styled(prefix.to_string(), Style::default().fg(Color::Yellow)),
            Span::styled(format!("#{} ", t.id), Style::default().fg(Color::DarkGray)),
            Span::styled(t.title.clone(), row_style),
            Span::styled(
                format!(" \u{00b7} ERR {}/{}", t.iteration, t.max_iterations),
                Style::default().fg(Color::Red),
            ),
        ]));
    }

    // --- Action bar at bottom (only when focused) ---
    if is_focused {
        // Find selected ticket status
        let selected_status = tickets.iter()
            .find(|t| t.id == app.board_selected)
            .map(|t| t.status);

        let mut actions: Vec<Span> = Vec::new();
        actions.push(Span::styled(" ", Style::default()));

        // Always show Enter to view detail
        actions.push(Span::styled("[Enter]", Style::default().fg(Color::Yellow)));
        actions.push(Span::styled(" View ", Style::default().fg(Color::DarkGray)));

        // Context-sensitive actions based on selected ticket status
        match selected_status {
            Some(TicketStatus::Error) => {
                actions.push(Span::styled("[r]", Style::default().fg(Color::Yellow)));
                actions.push(Span::styled(" Retry ", Style::default().fg(Color::DarkGray)));
                actions.push(Span::styled("[d]", Style::default().fg(Color::Yellow)));
                actions.push(Span::styled(" Del ", Style::default().fg(Color::DarkGray)));
                actions.push(Span::styled("[f]", Style::default().fg(Color::Yellow)));
                actions.push(Span::styled(" Files ", Style::default().fg(Color::DarkGray)));
            }
            Some(TicketStatus::Done) => {
                actions.push(Span::styled("[d]", Style::default().fg(Color::Yellow)));
                actions.push(Span::styled(" Del ", Style::default().fg(Color::DarkGray)));
                actions.push(Span::styled("[f]", Style::default().fg(Color::Yellow)));
                actions.push(Span::styled(" Files ", Style::default().fg(Color::DarkGray)));
            }
            Some(TicketStatus::Working) => {
                actions.push(Span::styled("[f]", Style::default().fg(Color::Yellow)));
                actions.push(Span::styled(" Files ", Style::default().fg(Color::DarkGray)));
            }
            _ => {}
        }

        // Always show batch clear
        let has_completed = tickets.iter().any(|t| matches!(t.status, TicketStatus::Done | TicketStatus::Error));
        if has_completed {
            actions.push(Span::styled("[D]", Style::default().fg(Color::Yellow)));
            actions.push(Span::styled(" Clear All ", Style::default().fg(Color::DarkGray)));
        }

        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(actions));
    }

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// Compact single-line ticket for Queued/Done sections
fn ticket_compact_line<'a>(ticket: &TicketSnapshot, app: &App, show_elapsed: bool) -> Line<'a> {
    let sel = app.right_panel_focused && app.board_selected == ticket.id;
    let prefix = if sel { "\u{25b6} " } else { "  " };
    let row_style = if sel {
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    let mut spans = vec![
        Span::styled(prefix.to_string(), Style::default().fg(Color::Yellow)),
        Span::styled(format!("#{} ", ticket.id), Style::default().fg(Color::DarkGray)),
        Span::styled(ticket.title.clone(), row_style),
    ];
    if show_elapsed && ticket.elapsed_secs > 0 {
        spans.push(Span::styled(
            format!(" \u{00b7} {}", format_elapsed(ticket.elapsed_secs)),
            Style::default().fg(Color::Green),
        ));
    }
    Line::from(spans)
}

/// Short display for TeamMode
fn team_mode_short(mode: &TeamMode) -> &str {
    match mode {
        TeamMode::TechLeadTeam => "TechLead",
        TeamMode::Solo => "Solo",
        TeamMode::Custom(_) => "Custom",
    }
}


/// Popup detail view for a selected ticket.
///
/// Single scrollable area with Scrollbar:
///   Header (Status / Model / Team / Context / Criteria)
///   → Prompt (word-wrapped)
///   → Summary (Done/Error only, before log)
///   → Running Log (extracted from vt100 parser screen)
fn draw_board_detail_popup(frame: &mut Frame, app: &App) {
    let popup_area = centered_rect(80, 80, frame.area());
    frame.render_widget(Clear, popup_area);

    let ticket = app.ticket_snapshot.as_ref()
        .and_then(|ts| ts.iter().find(|t| t.id == app.board_selected));

    let title = match ticket {
        Some(t) => format!(" #{} {} ", t.id, t.title),
        None => " Ticket Detail ".to_string(),
    };

    let block = Block::default()
        .title(title)
        .title_bottom(Line::from(" [Esc: close] [j/k: scroll] ").centered())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));

    let inner = block.inner(popup_area);

    let Some(t) = ticket else {
        let content = Paragraph::new("  No ticket selected")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(content, popup_area);
        return;
    };

    let wrap_w = inner.width.saturating_sub(4) as usize; // leave room for scrollbar + padding
    let mut lines: Vec<Line> = Vec::new();

    // --- Header ---

    // Status
    let status_str = match t.status {
        TicketStatus::Working => {
            let w = t.assigned_worker.map(|w| format!("W{}", w)).unwrap_or_default();
            format!("Working \u{00b7} {} \u{00b7} iter {}/{} \u{00b7} {}",
                w, t.iteration, t.max_iterations, format_elapsed(t.elapsed_secs))
        }
        TicketStatus::Queued => "Queued".to_string(),
        TicketStatus::Done => format!("Done \u{00b7} {}", format_elapsed(t.elapsed_secs)),
        TicketStatus::Error => format!("Error \u{00b7} iter {}/{} \u{00b7} {}",
            t.iteration, t.max_iterations, format_elapsed(t.elapsed_secs)),
    };
    lines.push(Line::from(vec![
        Span::styled(" Status: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(status_str, Style::default().fg(Color::White)),
    ]));

    // Model: Provider / Model (from worker pane)
    let pane_idx = t.assigned_worker.map(|w| w as usize);
    if let Some(idx) = pane_idx {
        if let Some(pane) = app.panes.get(idx) {
            let provider_name = pane.current_provider
                .and_then(|pi| app.providers.get(pi))
                .map(|p| p.name.as_str())
                .unwrap_or("Default");
            let model_name = pane.current_model.as_deref().unwrap_or("(none)");
            lines.push(Line::from(vec![
                Span::styled(" Model: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::styled(format!("{} / {}", provider_name, model_name), Style::default().fg(Color::White)),
            ]));
        }
    }

    // Team: show members
    let team_str = match &t.team_mode {
        TeamMode::TechLeadTeam => "TechLeader \u{00b7} Engineer \u{00b7} QA",
        TeamMode::Solo => "Solo",
        TeamMode::Custom(s) => s.as_str(),
    };
    lines.push(Line::from(vec![
        Span::styled(" Team: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::styled(team_str, Style::default().fg(Color::White)),
    ]));

    // Context
    if let Some(ref ctx) = t.context {
        lines.push(Line::from(vec![
            Span::styled(" Context: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(ctx.as_str(), Style::default().fg(Color::White)),
        ]));
    }

    // Criteria
    if let Some(ref crit) = t.criteria {
        lines.push(Line::from(vec![
            Span::styled(" Criteria: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(crit.as_str(), Style::default().fg(Color::White)),
        ]));
    }

    // --- Prompt (word-wrapped) ---
    if !t.prompt.is_empty() {
        let sep_w = inner.width.saturating_sub(2) as usize;
        lines.push(Line::from(Span::raw("")));
        lines.push(Line::from(Span::styled(
            format!(" \u{2500}\u{2500}\u{2500} Prompt {}", "\u{2500}".repeat(sep_w.saturating_sub(12))),
            Style::default().fg(Color::DarkGray),
        )));
        for wl in wrap_to_lines(&t.prompt, wrap_w) {
            lines.push(Line::from(Span::styled(format!(" {}", wl), Style::default().fg(Color::White))));
        }
    }

    // --- Summary (Done/Error only — placed before Running Log) ---
    if matches!(t.status, TicketStatus::Done | TicketStatus::Error) {
        if let Some(ref summary) = t.summary {
            let sep_w = inner.width.saturating_sub(2) as usize;
            lines.push(Line::from(Span::raw("")));
            lines.push(Line::from(Span::styled(
                format!(" \u{2500}\u{2500}\u{2500} Summary {}", "\u{2500}".repeat(sep_w.saturating_sub(13))),
                Style::default().fg(Color::DarkGray),
            )));
            let cleaned = clean_xml_tags(summary);
            for wl in wrap_to_lines(&cleaned, wrap_w) {
                lines.push(Line::from(Span::styled(format!(" {}", wl), Style::default().fg(Color::White))));
            }
        }
    }

    // --- Running Log (from per-ticket log buffer) ---
    if let Some(buf) = app.ticket_logs.get(&t.id) {
        if let Ok(log_lines) = buf.lock() {
            if !log_lines.is_empty() {
                let sep_w = inner.width.saturating_sub(2) as usize;
                lines.push(Line::from(Span::raw("")));
                lines.push(Line::from(Span::styled(
                    format!(" \u{2500}\u{2500}\u{2500} Running Log {}", "\u{2500}".repeat(sep_w.saturating_sub(16))),
                    Style::default().fg(Color::DarkGray),
                )));

                let mut trailing_empty = 0usize;
                for log_line in log_lines.iter() {
                    let trimmed = log_line.trim_end();
                    if trimmed.is_empty() {
                        trailing_empty += 1;
                    } else {
                        if trailing_empty > 0 {
                            lines.push(Line::from(Span::raw("")));
                        }
                        trailing_empty = 0;
                        lines.push(Line::from(Span::styled(
                            format!(" {}", trimmed),
                            Style::default().fg(Color::Gray),
                        )));
                    }
                }
            }
        }
    }

    // --- Render: single scrollable Paragraph + Scrollbar ---
    let total_lines = lines.len();
    let visible_height = inner.height as usize;
    let scroll = app.board_detail_scroll;

    // Clamp scroll to valid range
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll_clamped = scroll.min(max_scroll);

    let paragraph = Paragraph::new(lines)
        .scroll((scroll_clamped as u16, 0))
        .block(block);
    frame.render_widget(paragraph, popup_area);

    // Scrollbar (right side)
    if total_lines > visible_height {
        let mut scrollbar_state = ScrollbarState::new(max_scroll)
            .position(scroll_clamped);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("\u{25b2}"))
            .end_symbol(Some("\u{25bc}"));
        // Render scrollbar inside the block's inner area
        frame.render_stateful_widget(scrollbar, inner, &mut scrollbar_state);
    }
}

/// Strip XML-like tags from content (e.g. `<tool_call_begin>`, `<param name="...">`)
fn clean_xml_tags(s: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for ch in s.chars() {
        if ch == '<' { in_tag = true; continue; }
        if ch == '>' { in_tag = false; continue; }
        if !in_tag { result.push(ch); }
    }
    // Collapse multiple whitespace into single space
    let mut prev_space = false;
    let cleaned: String = result.chars().filter(|c| {
        if *c == ' ' {
            if prev_space { return false; }
            prev_space = true;
        } else {
            prev_space = false;
        }
        true
    }).collect();
    cleaned.trim().to_string()
}

/// Word-wrap a string into lines of at most `width` characters.
fn wrap_to_lines(s: &str, width: usize) -> Vec<String> {
    if width == 0 { return vec![s.to_string()]; }
    let mut out = Vec::new();
    for line in s.lines() {
        if line.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut remaining = line;
        while !remaining.is_empty() {
            if remaining.len() <= width {
                out.push(remaining.to_string());
                break;
            }
            let break_at = remaining[..width]
                .rfind(|c: char| c.is_whitespace())
                .unwrap_or(width);
            let break_at = if break_at == 0 { width } else { break_at };
            out.push(remaining[..break_at].to_string());
            remaining = remaining[break_at..].trim_start();
        }
    }
    out
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let mode_hint = if app.is_squad() && app.mode == AppMode::Normal {
        vec![
            Span::styled(" Alt+\u{2190}\u{2192}", Style::default().fg(Color::Yellow)),
            Span::styled(": Focus ", Style::default().fg(Color::DarkGray)),
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
                PopupMenu::NewSessionInput => vec![
                    Span::styled(" Enter", Style::default().fg(Color::Yellow)),
                    Span::styled(": Create ", Style::default().fg(Color::DarkGray)),
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
    let (pw, ph) = match menu {
        PopupMenu::RetryForm => (70, 80),
        PopupMenu::DeleteConfirm | PopupMenu::ClearConfirm => (50, 30),
        PopupMenu::SessionDeleteConfirm => (55, 40),
        PopupMenu::CompleteRecordChoice => (55, 25),
        PopupMenu::FileDiff => (95, 90),
        _ => (60, 60),
    };
    let area = centered_rect(pw, ph, frame.area());
    frame.render_widget(Clear, area);

    match menu {
        PopupMenu::Main => draw_main_menu(frame, app, area),
        PopupMenu::Matrix => draw_matrix(frame, app, area),
        PopupMenu::Provider => draw_provider_menu(frame, app, area),
        PopupMenu::Model => draw_model_menu(frame, app, area),
        PopupMenu::SessionList => draw_session_list(frame, app, area),
        PopupMenu::CompleteSession => draw_complete_session(frame, app, area),
        PopupMenu::NewSessionInput => draw_new_session_input(frame, app, area),
        PopupMenu::RemoveWorkerList => draw_remove_worker_list(frame, app, area),
        PopupMenu::RemoveWorkerConfirm => draw_remove_worker_confirm(frame, app, area),
        PopupMenu::ConnectProvider => draw_connect_provider(frame, app, area),
        PopupMenu::ProviderApiKeyInput => draw_api_key_input(frame, app, area),
        PopupMenu::MaxRetries => draw_max_retries(frame, app, area),
        PopupMenu::RetryForm => draw_retry_form(frame, app, area),
        PopupMenu::DeleteConfirm => draw_delete_confirm(frame, app, area),
        PopupMenu::ClearConfirm => draw_clear_confirm(frame, app, area),
        PopupMenu::SessionDeleteConfirm => draw_session_delete_confirm(frame, app, area),
        PopupMenu::CompleteRecordChoice => draw_complete_record_choice(frame, app, area),
        PopupMenu::FileDiff => draw_file_diff(frame, app, area),
    }
}

fn draw_main_menu(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Legion [ESC] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::DarkGray));

    let menu_items = app.main_menu_items();
    let items: Vec<ListItem> = menu_items
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
                MainMenuItem::AddWorker => {
                    let wc = app.worker_count();
                    format!("[{}/{}]", wc, crate::app::MAX_WORKERS)
                }
                MainMenuItem::RemoveWorker => {
                    format!("[{} worker{}]", app.worker_count(), if app.worker_count() == 1 { "" } else { "s" })
                }
                MainMenuItem::SwitchSession => {
                    format!("[{}]", app.session_name())
                }
                MainMenuItem::ConnectProvider => {
                    let connected = app.providers.len();
                    format!("[{} connected]", connected)
                }
                MainMenuItem::MaxRetries => {
                    format!("[{}]", app.default_max_iterations)
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
        .title(" Sessions ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::DarkGray));

    let mut items: Vec<ListItem> = Vec::new();
    let current_name = app.current_session.as_ref().map(|s| s.name.as_str());

    // Collect active and completed sessions separately
    // Active: default first, then others
    let mut active: Vec<_> = app.session_list.iter().enumerate()
        .filter(|(_, s)| s.status == "active")
        .collect();
    active.sort_by(|(_, a), (_, b)| b.is_default.cmp(&a.is_default));

    let completed: Vec<_> = app.session_list.iter().enumerate()
        .filter(|(_, s)| s.status == "completed")
        .collect();

    // "Active:" header
    items.push(ListItem::new(Line::from(Span::styled(
        "  Active:", Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
    ))));

    for (i, session) in &active {
        let selected = *i == app.session_list_index;
        let prefix = if selected { "> " } else { "  " };
        let icon = if current_name == Some(session.name.as_str()) {
            "\u{25cf} "
        } else {
            "\u{25cb} "
        };
        let style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        let mut spans = vec![
            Span::raw(prefix.to_string()),
            Span::styled(icon, style),
            Span::styled(session.name.clone(), style),
        ];
        if session.is_default {
            spans.push(Span::styled(" [default]", Style::default().fg(Color::Cyan)));
        }
        spans.push(Span::styled(
            format!("  {} workers", session.worker_count),
            Style::default().fg(Color::DarkGray),
        ));
        items.push(ListItem::new(Line::from(spans)));
    }

    // Completed section
    if !completed.is_empty() {
        items.push(ListItem::new("")); // spacer
        items.push(ListItem::new(Line::from(Span::styled(
            "  Completed:", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD),
        ))));

        for (i, session) in &completed {
            let selected = *i == app.session_list_index;
            let prefix = if selected { "> " } else { "  " };
            let style = if selected {
                Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let date = format_relative_time(session.completed_at.unwrap_or(session.created_at));
            items.push(ListItem::new(Line::from(vec![
                Span::raw(prefix.to_string()),
                Span::styled("\u{2713} ", style),
                Span::styled(session.name.clone(), style),
                Span::styled(format!("  {}", date), Style::default().fg(Color::DarkGray)),
            ])));
        }
    }

    // "New Session" entry
    items.push(ListItem::new("")); // spacer
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

    // Footer hints
    items.push(ListItem::new("")); // spacer
    items.push(ListItem::new(Line::from(vec![
        Span::styled("  [Enter", Style::default().fg(Color::DarkGray)),
        Span::styled("=Resume] ", Style::default().fg(Color::DarkGray)),
        Span::styled("[n", Style::default().fg(Color::DarkGray)),
        Span::styled("=New] ", Style::default().fg(Color::DarkGray)),
        Span::styled("[d", Style::default().fg(Color::DarkGray)),
        Span::styled("=Delete] ", Style::default().fg(Color::DarkGray)),
        Span::styled("[c", Style::default().fg(Color::DarkGray)),
        Span::styled("=Complete]", Style::default().fg(Color::DarkGray)),
    ])));
    items.push(ListItem::new(Line::from(Span::styled(
        "  [x=Remove completed]",
        Style::default().fg(Color::DarkGray),
    ))));

    frame.render_widget(List::new(items).block(block), area);
}

fn format_relative_time(ts: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let diff = now.saturating_sub(ts as u64);
    if diff < 86400 { "today".to_string() }
    else if diff < 172800 { "yesterday".to_string() }
    else { format!("{}d ago", diff / 86400) }
}

fn draw_new_session_input(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" New Session [ESC] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green))
        .style(Style::default().bg(Color::DarkGray));

    let items = vec![
        ListItem::new(Line::from(Span::styled(
            "  Enter session name:",
            Style::default().fg(Color::White),
        ))),
        ListItem::new(Line::from(Span::raw(""))),
        ListItem::new(Line::from(vec![
            Span::styled("  > ", Style::default().fg(Color::Yellow)),
            Span::styled(
                app.session_name_input.as_str(),
                Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::styled("\u{2588}", Style::default().fg(Color::Yellow)), // block cursor
        ])),
        ListItem::new(Line::from(Span::raw(""))),
        ListItem::new(Line::from(Span::styled(
            "  (a-z, 0-9, hyphens, underscores)",
            Style::default().fg(Color::DarkGray),
        ))),
    ];

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

fn draw_remove_worker_list(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Remove Worker [ESC] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .style(Style::default().bg(Color::DarkGray));

    let mut items: Vec<ListItem> = Vec::new();

    // Workers only (skip leader at index 0)
    for (wi, pane) in app.panes.iter().enumerate().skip(1) {
        let selected = (wi - 1) == app.remove_worker_target;
        let prefix = if selected { "> " } else { "  " };
        let model = pane.current_model.as_deref().unwrap_or("--");
        let style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        items.push(ListItem::new(Line::from(vec![
            Span::raw(prefix),
            Span::styled(&pane.label, style),
            Span::styled(format!("  [{}]", model), Style::default().fg(Color::DarkGray)),
        ])));
    }

    frame.render_widget(List::new(items).block(block), area);
}

fn draw_remove_worker_confirm(frame: &mut Frame, app: &App, area: Rect) {
    let worker_label = app.panes.get(app.remove_worker_target + 1)
        .map(|p| p.label.as_str())
        .unwrap_or("Worker");

    let block = Block::default()
        .title(format!(" Remove '{}' [ESC] ", worker_label))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .style(Style::default().bg(Color::DarkGray));

    let options = ["Merge to main", "Keep worktree", "Discard changes"];
    let descriptions = [
        "Merge branch into main, then remove worktree",
        "Keep worktree for manual handling later",
        "Delete worktree and branch (destructive!)",
    ];

    let items: Vec<ListItem> = options.iter().zip(descriptions.iter()).enumerate()
        .map(|(i, (opt, desc))| {
            let selected = i == app.remove_worker_strategy_index;
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
    } else if secs < 3600 {
        format!("{}m{}s", secs / 60, secs % 60)
    } else {
        format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Centered rectangle helper
fn draw_connect_provider(frame: &mut Frame, app: &App, area: Rect) {
    use crate::app::PROVIDER_TEMPLATES;
    let block = Block::default()
        .title(" Connect Provider [ESC] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::DarkGray));

    let items: Vec<ListItem> = PROVIDER_TEMPLATES
        .iter()
        .enumerate()
        .map(|(i, tmpl)| {
            let selected = i == app.connect_provider_index;
            let prefix = if selected { "> " } else { "  " };
            let connected = app.is_provider_connected(tmpl.id);
            let badge = if connected { " \u{25cf}" } else { "" }; // ● dot

            let style = if selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else if connected {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!("{}{}", prefix, tmpl.name), style),
                Span::styled(badge, Style::default().fg(Color::Green)),
                Span::styled(format!("  {}", tmpl.api_format), Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

fn draw_api_key_input(frame: &mut Frame, app: &App, area: Rect) {
    use crate::app::PROVIDER_TEMPLATES;
    let tmpl = &PROVIDER_TEMPLATES[app.connect_provider_index];

    let block = Block::default()
        .title(format!(" {} - API Key [ESC] ", tmpl.name))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("  Provider: {}", tmpl.name),
        Style::default().fg(Color::White),
    )));
    lines.push(Line::from(Span::styled(
        format!("  Base URL: {}", tmpl.base_url),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        format!("  Format:   {}", tmpl.api_format),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::styled(
        format!("  Env var:  ${}", tmpl.env_var),
        Style::default().fg(Color::DarkGray),
    )));
    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        "  API Key:",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    )));

    // Show masked key with cursor
    let masked: String = if app.api_key_input.is_empty() {
        "\u{2588}".to_string() // block cursor
    } else {
        let visible_len = app.api_key_input.len().min(4);
        let masked_len = app.api_key_input.len().saturating_sub(4);
        format!("{}{}\u{2588}",
            "\u{2022}".repeat(masked_len), // bullets
            &app.api_key_input[masked_len..masked_len + visible_len], // last 4 chars visible
        )
    };
    lines.push(Line::from(Span::styled(
        format!("  {}", masked),
        Style::default().fg(Color::Yellow),
    )));

    lines.push(Line::from(Span::raw("")));
    lines.push(Line::from(Span::styled(
        "  [Enter] Save  [Esc] Cancel",
        Style::default().fg(Color::DarkGray),
    )));

    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_max_retries(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(format!(" Max Retries (current: {}) [ESC] ", app.default_max_iterations))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::DarkGray));

    let items: Vec<ListItem> = (1..=10)
        .map(|n| {
            let selected = app.submenu_index == (n - 1);
            let is_current = n as u16 == app.default_max_iterations;
            let prefix = if selected { "> " } else { "  " };
            let badge = if is_current { " \u{25cf}" } else { "" };

            let style = if selected {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else if is_current {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::White)
            };

            ListItem::new(Line::from(vec![
                Span::styled(format!("{}{} retries", prefix, n), style),
                Span::styled(badge, Style::default().fg(Color::Green)),
            ]))
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

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

fn draw_retry_form(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(format!(" Retry Ticket #{} [ESC] ", app.retry_target_id))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let field_labels = ["Prompt", "Context", "Criteria", "Feedback"];
    let mut lines: Vec<Line> = Vec::new();

    for (i, label) in field_labels.iter().enumerate() {
        let is_focused = app.retry_form_focus as usize == i;
        let label_style = if is_focused {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(Span::styled(
            format!("  {}:", label),
            label_style,
        )));

        // Show field value (truncated to fit) with cursor if focused
        let val = &app.retry_form_fields[i];
        let max_width = inner.width.saturating_sub(6) as usize;
        let display = if val.len() > max_width {
            format!("...{}", &val[val.len() - max_width + 3..])
        } else {
            val.clone()
        };
        let field_text = if is_focused {
            format!("  {}\u{2588}", display)
        } else {
            format!("  {}", display)
        };
        let border_style = if is_focused {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        lines.push(Line::from(Span::styled(field_text, border_style)));
        lines.push(Line::from(Span::raw("")));
    }

    lines.push(Line::from(vec![
        Span::styled("  [Tab]", Style::default().fg(Color::Yellow)),
        Span::styled(" Switch  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
        Span::styled(" Confirm  ", Style::default().fg(Color::DarkGray)),
        Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
        Span::styled(" Cancel", Style::default().fg(Color::DarkGray)),
    ]));

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

fn draw_delete_confirm(frame: &mut Frame, app: &App, area: Rect) {
    let title = app.ticket_snapshot.as_ref()
        .and_then(|ts| ts.iter().find(|t| t.id == app.delete_confirm_id))
        .map(|t| t.title.clone())
        .unwrap_or_else(|| format!("#{}", app.delete_confirm_id));

    let block = Block::default()
        .title(format!(" Delete Ticket #{} ", app.delete_confirm_id))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .style(Style::default().bg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            format!("  Delete \"{}\" ?", truncate_str(&title, inner.width.saturating_sub(14) as usize)),
            Style::default().fg(Color::White),
        )),
        Line::from(Span::raw("")),
        Line::from(vec![
            Span::styled("  [Enter/y]", Style::default().fg(Color::Yellow)),
            Span::styled(" Confirm  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
            Span::styled(" Cancel", Style::default().fg(Color::DarkGray)),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_clear_confirm(frame: &mut Frame, app: &App, area: Rect) {
    let count = app.ticket_snapshot.as_ref()
        .map(|ts| ts.iter().filter(|t| matches!(t.status, legion_core::orchestrate::engine::TicketStatus::Done | legion_core::orchestrate::engine::TicketStatus::Error)).count())
        .unwrap_or(0);

    let block = Block::default()
        .title(" Clear Completed ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .style(Style::default().bg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            format!("  Clear {} completed/failed ticket{} ?", count, if count == 1 { "" } else { "s" }),
            Style::default().fg(Color::White),
        )),
        Line::from(Span::raw("")),
        Line::from(vec![
            Span::styled("  [Enter/y]", Style::default().fg(Color::Yellow)),
            Span::styled(" Confirm  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
            Span::styled(" Cancel", Style::default().fg(Color::DarkGray)),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_session_delete_confirm(frame: &mut Frame, app: &App, area: Rect) {
    let name = app.session_delete_target.as_deref().unwrap_or("?");
    let block = Block::default()
        .title(" Delete Session ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .style(Style::default().bg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            format!("  Delete session \"{}\"?", name),
            Style::default().fg(Color::White),
        )),
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            format!("  Tickets: {}  Pending: {}  Logs: {}",
                app.session_delete_ticket_count,
                app.session_delete_pending_count,
                app.session_delete_log_count),
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::raw("")),
        Line::from(vec![
            Span::styled("  [Enter/y]", Style::default().fg(Color::Yellow)),
            Span::styled(" Confirm  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[Esc/n]", Style::default().fg(Color::Yellow)),
            Span::styled(" Cancel", Style::default().fg(Color::DarkGray)),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_complete_record_choice(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Session Records ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let choices = ["Discard tickets & logs", "Migrate to default session"];
    let mut lines = vec![
        Line::from(Span::raw("")),
        Line::from(Span::styled(
            "  What to do with session records?",
            Style::default().fg(Color::White),
        )),
        Line::from(Span::raw("")),
    ];
    for (i, label) in choices.iter().enumerate() {
        let prefix = if i == app.complete_record_choice { "▸ " } else { "  " };
        let style = if i == app.complete_record_choice {
            Style::default().fg(Color::Cyan)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        lines.push(Line::from(Span::styled(format!("  {}{}", prefix, label), style)));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

fn draw_file_diff(frame: &mut Frame, app: &App, area: Rect) {
    // Find ticket title for header
    let ticket_title = app.ticket_snapshot.as_ref()
        .and_then(|ts| ts.iter().find(|t| t.id == app.diff_ticket_id))
        .map(|t| format!(" #{} {} ", t.id, truncate_str(&t.title, 40)))
        .unwrap_or_else(|| format!(" #{} ", app.diff_ticket_id));

    // Loading state
    if app.diff_loading {
        let block = Block::default()
            .title(ticket_title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));
        let p = Paragraph::new("  Loading diff...").block(block);
        frame.render_widget(p, area);
        return;
    }

    // Error state
    if let Some(ref err) = app.diff_error {
        let block = Block::default()
            .title(ticket_title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red));
        let p = Paragraph::new(format!("  Error: {}", err)).block(block);
        frame.render_widget(p, area);
        return;
    }

    let Some(ref data) = app.diff_data else {
        let block = Block::default()
            .title(ticket_title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));
        let p = Paragraph::new("  No diff data").block(block);
        frame.render_widget(p, area);
        return;
    };

    if data.files.is_empty() {
        let block = Block::default()
            .title(ticket_title)
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));
        let p = Paragraph::new("  No file changes").block(block);
        frame.render_widget(p, area);
        return;
    }

    // Compute total stats
    let total_add: usize = data.files.iter().map(|f| f.additions).sum();
    let total_del: usize = data.files.iter().map(|f| f.deletions).sum();

    // Outer block
    let outer_block = Block::default()
        .title(format!("{}\u{2500}\u{2500} {} files changed, +{} -{} ",
            ticket_title, data.files.len(), total_add, total_del))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));
    let inner = outer_block.inner(area);
    frame.render_widget(outer_block, area);

    // Split: left 35% (file list) | right 65% (diff content)
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(35),
            Constraint::Percentage(65),
        ])
        .split(inner);

    let left_area = h_chunks[0];
    let right_area = h_chunks[1];

    // === Left panel: file list ===
    let file_block = Block::default()
        .title(" Files ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let file_inner = file_block.inner(left_area);

    let file_items: Vec<ListItem> = data.files.iter().enumerate().map(|(i, f)| {
        let status_color = match f.status.as_str() {
            "A" => Color::Green,
            "D" => Color::Red,
            _ => Color::Yellow,
        };
        let prefix = if i == app.diff_file_selected { "\u{25b6} " } else { "  " };
        let stats = format!("+{} -{}", f.additions, f.deletions);

        // Truncate path to fit
        let max_path_len = file_inner.width.saturating_sub(stats.len() as u16 + 5) as usize;
        let display_path = if f.path.len() > max_path_len && max_path_len > 3 {
            format!("...{}", &f.path[f.path.len().saturating_sub(max_path_len - 3)..])
        } else {
            f.path.clone()
        };

        let style = if i == app.diff_file_selected {
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };

        ListItem::new(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(
                format!("{}", f.status),
                Style::default().fg(status_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", style),
            Span::styled(display_path, style),
            Span::styled(
                format!(" {}", stats),
                Style::default().fg(Color::DarkGray),
            ),
        ]))
    }).collect();

    let file_list = List::new(file_items);
    frame.render_widget(file_block, left_area);
    frame.render_widget(file_list, file_inner);

    // File list scrollbar
    if data.files.len() > file_inner.height as usize {
        let max_scroll = data.files.len().saturating_sub(file_inner.height as usize);
        let scroll_pos = app.diff_file_selected.min(max_scroll);
        let mut sb_state = ScrollbarState::new(max_scroll).position(scroll_pos);
        let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None);
        frame.render_stateful_widget(sb, file_inner, &mut sb_state);
    }

    // === Right panel: diff content ===
    let selected_file = data.files.get(app.diff_file_selected);
    let diff_block = Block::default()
        .title(format!(" {} ", selected_file.map(|f| f.path.as_str()).unwrap_or("...")))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));
    let diff_inner = diff_block.inner(right_area);

    let diff_lines: Vec<Line> = selected_file
        .map(|f| {
            f.diff_lines.iter().map(|line| {
                if line.starts_with('+') && !line.starts_with("+++") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Green)))
                } else if line.starts_with('-') && !line.starts_with("---") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Red)))
                } else if line.starts_with("@@") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::Cyan)))
                } else if line.starts_with("diff --git") || line.starts_with("index ") || line.starts_with("---") || line.starts_with("+++") {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::DarkGray)))
                } else {
                    Line::from(Span::styled(line.clone(), Style::default().fg(Color::White)))
                }
            }).collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let total_diff_lines = diff_lines.len();
    let visible_h = diff_inner.height as usize;
    let max_scroll = total_diff_lines.saturating_sub(visible_h);
    let scroll_clamped = app.diff_scroll.min(max_scroll);

    let diff_paragraph = Paragraph::new(diff_lines)
        .scroll((scroll_clamped as u16, 0))
        .block(diff_block);
    frame.render_widget(diff_paragraph, right_area);

    // Diff content scrollbar
    if total_diff_lines > visible_h {
        let mut sb_state = ScrollbarState::new(max_scroll).position(scroll_clamped);
        let sb = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("\u{25b2}"))
            .end_symbol(Some("\u{25bc}"));
        frame.render_stateful_widget(sb, diff_inner, &mut sb_state);
    }

    // Footer hints (render at bottom of outer area)
    let footer_area = Rect {
        x: area.x + 1,
        y: area.y + area.height.saturating_sub(1),
        width: area.width.saturating_sub(2),
        height: 1,
    };
    let footer = Line::from(vec![
        Span::styled(" \u{2191}\u{2193}", Style::default().fg(Color::Yellow)),
        Span::styled(" Files ", Style::default().fg(Color::DarkGray)),
        Span::styled("j/k", Style::default().fg(Color::Yellow)),
        Span::styled(" Scroll ", Style::default().fg(Color::DarkGray)),
        Span::styled("PgUp/PgDn", Style::default().fg(Color::Yellow)),
        Span::styled(" Page ", Style::default().fg(Color::DarkGray)),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::styled(" Close ", Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(footer), footer_area);
}
