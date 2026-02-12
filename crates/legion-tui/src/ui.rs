//! TUI rendering and UI components

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::app::{App, AppMode, MainMenuItem, PopupMenu};

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Main draw function
pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header
            Constraint::Min(0),    // Main content
            Constraint::Length(1), // Footer
        ])
        .split(frame.area());

    draw_header(frame, app, chunks[0]);
    draw_main(frame, app, chunks[1]);
    draw_footer(frame, chunks[2]);

    // Draw popup if in popup mode
    if let AppMode::Popup(menu) = app.mode {
        draw_popup(frame, app, menu);
    }
}

/// Draw header showing version and current provider/model
fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let provider_name = app
        .get_current_provider()
        .map(|p| p.name.as_str())
        .unwrap_or("No Provider");

    let model_name = app
        .current_model
        .as_deref()
        .unwrap_or("No Model");

    let connected_indicator = if app.provider_connected {
        Span::styled(" \u{25cf}", Style::default().fg(Color::Green))
    } else {
        Span::styled(" \u{25cb}", Style::default().fg(Color::DarkGray))
    };

    let header = Line::from(vec![
        Span::styled(
            format!("Legion v{}", VERSION),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::raw("        "),
        Span::styled("[", Style::default().fg(Color::DarkGray)),
        Span::styled(provider_name, Style::default().fg(Color::Yellow)),
        Span::styled(" \u{2192} ", Style::default().fg(Color::DarkGray)),
        Span::styled(model_name, Style::default().fg(Color::Magenta)),
        Span::styled("]", Style::default().fg(Color::DarkGray)),
        connected_indicator,
    ]);

    let paragraph = Paragraph::new(header);
    frame.render_widget(paragraph, area);
}

/// Draw main content area (PTY output)
fn draw_main(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Claude Code ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));

    // Show last lines of PTY output
    let lines: Vec<&str> = app.pty_output.lines().rev().take(100).collect();
    let text: Vec<Line> = lines.iter().rev().map(|l| Line::from(*l)).collect();

    let content = Paragraph::new(text)
        .block(block)
        .wrap(ratatui::widgets::Wrap { trim: false });

    frame.render_widget(content, area);
}

/// Draw footer with keyboard shortcuts
fn draw_footer(frame: &mut Frame, area: Rect) {
    let footer = Line::from(vec![
        Span::styled("Ctrl+P", Style::default().fg(Color::Yellow)),
        Span::styled(": \u{83dc}\u{5355} ", Style::default().fg(Color::DarkGray)),
        Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
        Span::styled("Ctrl+Q", Style::default().fg(Color::Yellow)),
        Span::styled(": \u{9000}\u{51fa}", Style::default().fg(Color::DarkGray)),
    ]);

    let paragraph = Paragraph::new(footer);
    frame.render_widget(paragraph, area);
}

/// Draw popup container
fn draw_popup(frame: &mut Frame, app: &App, menu: PopupMenu) {
    let area = centered_rect(50, 60, frame.area());

    // Clear the area first
    frame.render_widget(Clear, area);

    match menu {
        PopupMenu::Main => draw_main_menu(frame, app, area),
        PopupMenu::Provider => draw_provider_menu(frame, app, area),
        PopupMenu::Model => draw_model_menu(frame, app, area),
        PopupMenu::Session => draw_session_menu(frame, app, area),
    }
}

/// Draw main popup menu
fn draw_main_menu(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Legion [ESC] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let items: Vec<ListItem> = App::main_menu_items()
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let is_selected = i == app.menu_index;
            let prefix = if is_selected { "> " } else { "  " };

            // Build the right-side value display
            let value = match item {
                MainMenuItem::Provider => {
                    let name = app
                        .get_current_provider()
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "None".to_string());
                    let indicator = if app.provider_connected { " \u{25cf}" } else { "" };
                    format!("[{}{}]", name, indicator)
                }
                MainMenuItem::Model => {
                    let model = app.current_model.as_deref().unwrap_or("None");
                    format!("[{}]", model)
                }
                MainMenuItem::Session => {
                    let session = app
                        .get_current_session()
                        .and_then(|s| s.name.clone())
                        .unwrap_or_else(|| "None".to_string());
                    format!("[{}]", session)
                }
                MainMenuItem::Settings | MainMenuItem::Quit => String::new(),
            };

            let style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let content = if value.is_empty() {
                Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(item.label(), style),
                ])
            } else {
                // Calculate padding for alignment
                let label_len = prefix.len() + item.label().len();
                let padding = " ".repeat(20usize.saturating_sub(label_len));

                Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(item.label(), style),
                    Span::raw(padding),
                    Span::styled(value, Style::default().fg(Color::DarkGray)),
                ])
            };

            ListItem::new(content)
        })
        .collect();

    // Add separator before Settings
    let mut final_items = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        if i == 3 {
            // Before Settings
            final_items.push(ListItem::new(Line::from(Span::styled(
                "  \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}",
                Style::default().fg(Color::DarkGray),
            ))));
        }
        final_items.push(item);
    }

    let list = List::new(final_items).block(block);
    frame.render_widget(list, area);
}

/// Draw provider selection menu
fn draw_provider_menu(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Select Provider [ESC] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let items: Vec<ListItem> = app
        .providers
        .iter()
        .enumerate()
        .map(|(i, provider)| {
            let is_selected = i == app.submenu_index;
            let is_current = app.current_provider == Some(i);
            let prefix = if is_selected { "> " } else { "  " };
            let indicator = if is_current { " \u{25cf}" } else { "" };

            let style = if is_selected {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let content = Line::from(vec![
                Span::raw(prefix),
                Span::styled(&provider.name, style),
                Span::styled(indicator, Style::default().fg(Color::Green)),
            ]);

            ListItem::new(content)
        })
        .collect();

    if items.is_empty() {
        let empty_msg = Paragraph::new("No providers configured")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(empty_msg, area);
    } else {
        let list = List::new(items).block(block);
        frame.render_widget(list, area);
    }
}

/// Draw model selection menu
fn draw_model_menu(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Select Model [ESC] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let models = app.get_current_provider_models();

    if let Some(models) = models {
        let items: Vec<ListItem> = models
            .iter()
            .enumerate()
            .map(|(i, model)| {
                let is_selected = i == app.submenu_index;
                let is_current = app.current_model.as_deref() == Some(model);
                let prefix = if is_selected { "> " } else { "  " };
                let indicator = if is_current { " \u{25cf}" } else { "" };

                let style = if is_selected {
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };

                let content = Line::from(vec![
                    Span::raw(prefix),
                    Span::styled(model, style),
                    Span::styled(indicator, Style::default().fg(Color::Green)),
                ]);

                ListItem::new(content)
            })
            .collect();

        let list = List::new(items).block(block);
        frame.render_widget(list, area);
    } else {
        let empty_msg = Paragraph::new("No models available (select a provider first)")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(empty_msg, area);
    }
}

/// Draw session selection menu
fn draw_session_menu(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Select Session [ESC] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green));

    let items: Vec<ListItem> = app
        .sessions
        .iter()
        .enumerate()
        .map(|(i, session)| {
            let is_selected = i == app.submenu_index;
            let is_current = app.current_session == Some(i);
            let prefix = if is_selected { "> " } else { "  " };
            let indicator = if is_current { " \u{25cf}" } else { "" };

            let name = session
                .name
                .as_deref()
                .unwrap_or(&session.id);

            let style = if is_selected {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let content = Line::from(vec![
                Span::raw(prefix),
                Span::styled(name, style),
                Span::styled(indicator, Style::default().fg(Color::Green)),
            ]);

            ListItem::new(content)
        })
        .collect();

    if items.is_empty() {
        let empty_msg = Paragraph::new("No sessions available")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(empty_msg, area);
    } else {
        let list = List::new(items).block(block);
        frame.render_widget(list, area);
    }
}

/// Helper function to create a centered rect
pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
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
        .split(popup_layout[1])[1]
}
