//! Input handling - mode-aware key routing

use crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use crate::app::{App, AppMode, PopupMenu};

pub enum InputResult {
    Continue,
    Quit,
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> InputResult {
    // Global: Ctrl+Q always quits
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
        return InputResult::Quit;
    }

    match app.mode {
        AppMode::Normal => handle_normal_mode(app, key),
        AppMode::Popup(_) => handle_popup_mode(app, key),
    }
}

fn handle_normal_mode(app: &mut App, key: KeyEvent) -> InputResult {
    // Ctrl+P toggles popup menu
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
        app.toggle_popup();
        return InputResult::Continue;
    }

    // Task Board navigation when right panel is focused
    if app.right_panel_focused && app.is_squad() {
        if app.board_detail_open {
            // Detail popup is open: j/k scroll, Esc closes
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => {
                    app.board_detail_scroll = app.board_detail_scroll.saturating_add(1);
                    return InputResult::Continue;
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    app.board_detail_scroll = app.board_detail_scroll.saturating_sub(1);
                    return InputResult::Continue;
                }
                KeyCode::Esc => {
                    app.board_detail_open = false;
                    app.board_detail_scroll = 0;
                    return InputResult::Continue;
                }
                _ => {
                    return InputResult::Continue;
                }
            }
        }
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                navigate_ticket_down(app);
                return InputResult::Continue;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                navigate_ticket_up(app);
                return InputResult::Continue;
            }
            KeyCode::Enter => {
                app.board_detail_open = true;
                app.board_detail_scroll = 0;
                return InputResult::Continue;
            }
            KeyCode::Char('r') => {
                // Retry selected Error ticket
                if let Some(engine) = app.orchestrate.clone() {
                    let selected = app.board_selected;
                    tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(async {
                            if engine.retry_ticket(selected).await {
                                app.ticket_logs.remove(&selected);
                                tracing::info!("Retrying ticket {}", selected);
                            }
                        });
                    });
                }
                return InputResult::Continue;
            }
            KeyCode::Char('d') => {
                // Delete selected Done/Error ticket
                if let Some(engine) = app.orchestrate.clone() {
                    let selected = app.board_selected;
                    tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(async {
                            if engine.delete_ticket(selected).await {
                                app.ticket_logs.remove(&selected);
                                tracing::info!("Deleted ticket {}", selected);
                            }
                        });
                    });
                }
                return InputResult::Continue;
            }
            KeyCode::Char('D') => {
                // Clear all Done+Error tickets
                if let Some(engine) = app.orchestrate.clone() {
                    tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current().block_on(async {
                            let removed = engine.clear_completed().await;
                            for id in &removed {
                                app.ticket_logs.remove(id);
                            }
                            tracing::info!("Cleared {} completed tickets", removed.len());
                        });
                    });
                }
                return InputResult::Continue;
            }
            KeyCode::Esc | KeyCode::Left if key.code == KeyCode::Esc || key.modifiers.contains(KeyModifiers::ALT) => {
                app.right_panel_focused = false;
                return InputResult::Continue;
            }
            _ => {
                // Don't swallow Alt+key combos — let them fall through to squad shortcuts
                if key.modifiers.contains(KeyModifiers::ALT) || key.modifiers.contains(KeyModifiers::CONTROL) {
                    // fall through
                } else {
                    return InputResult::Continue;
                }
            }
        }
    }

    // Squad-only shortcuts (not forwarded to PTY)
    if app.is_squad() {
        match key.code {
            // Alt+Right / Alt+Left toggle focus between Leader and Task Board
            KeyCode::Right if key.modifiers.contains(KeyModifiers::ALT) => {
                app.right_panel_focused = !app.right_panel_focused;
                return InputResult::Continue;
            }
            KeyCode::Left if key.modifiers.contains(KeyModifiers::ALT) => {
                app.right_panel_focused = !app.right_panel_focused;
                return InputResult::Continue;
            }
            // Ctrl+Left/Right adjust leader/worker split ratio
            KeyCode::Left if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.adjust_leader_ratio(-5);
                return InputResult::Continue;
            }
            KeyCode::Right if key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.adjust_leader_ratio(5);
                return InputResult::Continue;
            }
            _ => {}
        }
    }

    // Everything else goes to PTY
    let bytes = key_to_bytes(key);
    if !bytes.is_empty() {
        app.write_to_pty(&bytes);
    }

    InputResult::Continue
}

/// Handle mouse events for divider hover and dragging (squad mode only)
pub fn handle_mouse(app: &mut App, event: MouseEvent) {
    if !app.is_squad() {
        return;
    }

    let (term_width, _) = app.term_size;
    if term_width == 0 {
        return;
    }

    // Divider x position (between leader and workers), offset by header row
    let divider_x = (term_width as u32 * app.leader_ratio as u32 / 100) as u16;
    let near_divider = event.column.abs_diff(divider_x) <= 2 && event.row >= 1;

    match event.kind {
        MouseEventKind::Moved => {
            app.hover_on_divider = near_divider;
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if near_divider {
                app.dragging_divider = true;
                app.hover_on_divider = true;
            }
        }
        MouseEventKind::Drag(MouseButton::Left) if app.dragging_divider => {
            let old_ratio = app.leader_ratio;
            app.set_leader_ratio_from_x(event.column);
            if app.leader_ratio != old_ratio {
                app.apply_resize();
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if app.dragging_divider {
                app.dragging_divider = false;
                app.set_leader_ratio_from_x(event.column);
                app.apply_resize();
            }
        }
        MouseEventKind::ScrollUp => {
            if app.board_detail_open {
                app.board_detail_scroll = app.board_detail_scroll.saturating_sub(3);
            } else if app.right_panel_focused {
                navigate_ticket_up(app);
            }
        }
        MouseEventKind::ScrollDown => {
            if app.board_detail_open {
                app.board_detail_scroll = app.board_detail_scroll.saturating_add(3);
            } else if app.right_panel_focused {
                navigate_ticket_down(app);
            }
        }
        _ => {}
    }
}

fn handle_popup_mode(app: &mut App, key: KeyEvent) -> InputResult {
    // Ctrl+P also closes popup (but not during startup with no panes)
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
        if !app.panes.is_empty() {
            app.toggle_popup();
        }
        return InputResult::Continue;
    }

    match app.mode {
        AppMode::Popup(PopupMenu::Matrix) => handle_matrix_keys(app, key),
        AppMode::Popup(PopupMenu::Main) => handle_main_menu_keys(app, key),
        AppMode::Popup(PopupMenu::Provider) | AppMode::Popup(PopupMenu::Model) => {
            handle_submenu_keys(app, key)
        }
        AppMode::Popup(PopupMenu::SessionList) => handle_session_list_keys(app, key),
        AppMode::Popup(PopupMenu::CompleteSession) => handle_complete_session_keys(app, key),
        AppMode::Popup(PopupMenu::NewSessionInput) => handle_new_session_input_keys(app, key),
        AppMode::Popup(PopupMenu::RemoveWorkerList) => handle_remove_worker_list_keys(app, key),
        AppMode::Popup(PopupMenu::RemoveWorkerConfirm) => handle_remove_worker_confirm_keys(app, key),
        _ => {}
    }

    InputResult::Continue
}

fn handle_matrix_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.mode = AppMode::Popup(PopupMenu::Main);
        }
        KeyCode::Up | KeyCode::Char('k') => app.menu_up(),
        KeyCode::Down | KeyCode::Char('j') => app.menu_down(),
        KeyCode::Tab | KeyCode::Left | KeyCode::Right
        | KeyCode::Char('h') | KeyCode::Char('l') => app.matrix_col_toggle(),
        KeyCode::Enter => {
            app.matrix_enter();
        }
        _ => {}
    }
}

fn handle_main_menu_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => app.toggle_popup(),
        KeyCode::Up | KeyCode::Char('k') => app.menu_up(),
        KeyCode::Down | KeyCode::Char('j') => app.menu_down(),
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => app.enter_submenu(),
        _ => {}
    }
}

fn handle_submenu_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => {
            app.back_to_matrix();
        }
        KeyCode::Up | KeyCode::Char('k') => app.menu_up(),
        KeyCode::Down | KeyCode::Char('j') => app.menu_down(),
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            app.select_submenu_item();
            update_proxy_config(app);
            save_pane_configs(app);
        }
        _ => {}
    }
}

fn handle_session_list_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            // Only allow Esc if panes exist (not in startup mode)
            if !app.panes.is_empty() {
                app.mode = AppMode::Popup(PopupMenu::Main);
            }
        }
        KeyCode::Up | KeyCode::Char('k') => app.menu_up(),
        KeyCode::Down | KeyCode::Char('j') => app.menu_down(),
        KeyCode::Enter => {
            if app.session_list_index >= app.session_list.len() {
                // "New Session" selected → show text input with default name
                app.session_name_input = app.default_session_name();
                app.mode = AppMode::Popup(PopupMenu::NewSessionInput);
            } else {
                // Resume existing session
                let session = app.session_list[app.session_list_index].clone();
                let workers = session.worker_count as u16;
                match app.start_session(&session.name, workers, true) {
                    Ok(()) => {
                        tracing::info!("Resumed session: {}", session.name);
                        update_proxy_config(app);
                        app.mode = AppMode::Normal;
                    }
                    Err(e) => {
                        tracing::error!("Failed to resume session '{}': {}", session.name, e);
                    }
                }
            }
        }
        _ => {}
    }
}

fn handle_complete_session_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.mode = AppMode::Popup(PopupMenu::Main);
        }
        KeyCode::Up | KeyCode::Char('k') => app.menu_up(),
        KeyCode::Down | KeyCode::Char('j') => app.menu_down(),
        KeyCode::Enter => {
            let strategy = match app.complete_merge_index {
                0 => "merge",
                1 => "keep",
                _ => "discard",
            };

            app.kill_all();

            match app.complete_current_session(strategy) {
                Ok(true) => {
                    tracing::info!("Session completed with strategy: {}", strategy);
                }
                Ok(false) => {
                    tracing::warn!("No active session to complete");
                }
                Err(e) => {
                    tracing::error!("Failed to complete session: {}", e);
                }
            }
            app.mode = AppMode::Normal;
            app.should_quit = true;
        }
        _ => {}
    }
}

fn handle_new_session_input_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            // Go back to session list
            app.session_name_input.clear();
            app.load_session_list();
            app.mode = AppMode::Popup(PopupMenu::SessionList);
            app.session_list_index = 0;
        }
        KeyCode::Enter => {
            let name = app.session_name_input.trim().to_string();
            if !name.is_empty() {
                let workers = app.requested_workers;
                match app.start_session(&name, workers, false) {
                    Ok(()) => {
                        tracing::info!("Created new session: {}", name);
                        app.session_name_input.clear();
                        update_proxy_config(app);
                        app.mode = AppMode::Normal;
                    }
                    Err(e) => {
                        tracing::error!("Failed to create session '{}': {}", name, e);
                    }
                }
            }
        }
        KeyCode::Backspace => {
            app.session_name_input.pop();
        }
        KeyCode::Char(c) => {
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && (c.is_alphanumeric() || c == '-' || c == '_')
            {
                app.session_name_input.push(c);
            }
        }
        _ => {}
    }
}

fn handle_remove_worker_list_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.mode = AppMode::Popup(PopupMenu::Main);
        }
        KeyCode::Up | KeyCode::Char('k') => app.menu_up(),
        KeyCode::Down | KeyCode::Char('j') => app.menu_down(),
        KeyCode::Enter => {
            // Proceed to confirm dialog
            app.remove_worker_strategy_index = 0;
            app.mode = AppMode::Popup(PopupMenu::RemoveWorkerConfirm);
        }
        _ => {}
    }
}

fn handle_remove_worker_confirm_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            // Back to worker list
            app.mode = AppMode::Popup(PopupMenu::RemoveWorkerList);
        }
        KeyCode::Up | KeyCode::Char('k') => app.menu_up(),
        KeyCode::Down | KeyCode::Char('j') => app.menu_down(),
        KeyCode::Enter => {
            let strategy = match app.remove_worker_strategy_index {
                0 => "merge",
                1 => "keep",
                _ => "discard",
            };
            // pane_index = remove_worker_target + 1 (skip leader)
            let pane_index = app.remove_worker_target + 1;
            app.pending_remove_worker = Some((pane_index, strategy.to_string()));
            app.mode = AppMode::Normal;
        }
        _ => {}
    }
}

fn navigate_ticket_down(app: &mut App) {
    if let Some(ref tickets) = app.ticket_snapshot {
        if tickets.is_empty() {
            return;
        }
        let ids: Vec<usize> = tickets.iter().map(|t| t.id).collect();
        let current_pos = ids.iter().position(|&id| id == app.board_selected).unwrap_or(0);
        let next = if current_pos + 1 < ids.len() {
            current_pos + 1
        } else {
            0
        };
        app.board_selected = ids[next];
    }
}

fn navigate_ticket_up(app: &mut App) {
    if let Some(ref tickets) = app.ticket_snapshot {
        if tickets.is_empty() {
            return;
        }
        let ids: Vec<usize> = tickets.iter().map(|t| t.id).collect();
        let current_pos = ids.iter().position(|&id| id == app.board_selected).unwrap_or(0);
        let prev = if current_pos > 0 {
            current_pos - 1
        } else {
            ids.len() - 1
        };
        app.board_selected = ids[prev];
    }
}

/// After provider/model selection, POST config to affected panes' control APIs
fn update_proxy_config(app: &App) {
    let configs: Vec<(u16, String, String, Option<String>, Option<String>)> = app
        .panes
        .iter()
        .filter_map(|pane| {
            let provider = pane
                .current_provider
                .and_then(|i| app.providers.get(i))?;
            // Default mode: no proxy involved, skip config update
            if provider.id == "__default__" {
                return None;
            }
            Some((
                pane.control_port,
                provider.base_url.clone(),
                provider.api_format.clone(),
                provider.api_key.clone(),
                pane.current_model.clone(),
            ))
        })
        .collect();

    if configs.is_empty() {
        return;
    }

    tokio::spawn(async move {
        let client = reqwest::Client::new();
        for (port, base_url, api_format, api_key, model) in configs {
            let body = serde_json::json!({
                "target_url": base_url,
                "api_format": api_format,
                "api_key": api_key,
                "model": model,
            });
            let _ = client
                .post(format!("http://127.0.0.1:{}/legion/config", port))
                .json(&body)
                .send()
                .await;
        }
    });
}

/// After provider/model selection, persist each pane's config to the database
fn save_pane_configs(app: &App) {
    if let Ok(repo) = legion_db::open_db() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        for pane in &app.panes {
            if let Some(provider) = pane.current_provider.and_then(|i| app.providers.get(i)) {
                let config = legion_db::PaneConfig {
                    pane_label: pane.label.clone(),
                    provider_id: provider.id.clone(),
                    model: pane.current_model.clone(),
                    updated_at: now,
                };
                if let Err(e) = repo.upsert_pane_config(&config) {
                    tracing::warn!("Failed to save pane config for '{}': {}", pane.label, e);
                }
            }
        }
    }
}

/// Convert crossterm KeyEvent to PTY-compatible bytes
fn key_to_bytes(key: KeyEvent) -> Vec<u8> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                vec![(c as u8).wrapping_sub(b'a').wrapping_add(1)]
            } else {
                c.to_string().into_bytes()
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![127],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        _ => vec![],
    }
}
