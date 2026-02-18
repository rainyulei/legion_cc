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
                // Open retry form for selected Error ticket
                if let Some(ref tickets) = app.ticket_snapshot {
                    if let Some(t) = tickets.iter().find(|t| t.id == app.board_selected) {
                        if t.status == legion_core::orchestrate::engine::TicketStatus::Error {
                            app.retry_target_id = t.id;
                            app.retry_form_fields = [
                                t.prompt.clone(),
                                t.context.clone().unwrap_or_default(),
                                t.criteria.clone().unwrap_or_default(),
                                String::new(), // feedback
                            ];
                            app.retry_form_focus = 3; // default focus on feedback
                            app.mode = AppMode::Popup(PopupMenu::RetryForm);
                        }
                    }
                }
                return InputResult::Continue;
            }
            KeyCode::Char('d') => {
                // Open delete confirm for selected Done/Error ticket
                if let Some(ref tickets) = app.ticket_snapshot {
                    if let Some(t) = tickets.iter().find(|t| t.id == app.board_selected) {
                        if matches!(t.status, legion_core::orchestrate::engine::TicketStatus::Done | legion_core::orchestrate::engine::TicketStatus::Error) {
                            app.delete_confirm_id = t.id;
                            app.mode = AppMode::Popup(PopupMenu::DeleteConfirm);
                        }
                    }
                }
                return InputResult::Continue;
            }
            KeyCode::Char('f') => {
                // Open file diff popup for Working/Done/Error ticket
                let ticket_info = app.ticket_snapshot.as_ref().and_then(|tickets| {
                    tickets.iter().find(|t| t.id == app.board_selected).and_then(|t| {
                        if matches!(t.status,
                            legion_core::orchestrate::engine::TicketStatus::Working
                            | legion_core::orchestrate::engine::TicketStatus::Done
                            | legion_core::orchestrate::engine::TicketStatus::Error
                        ) {
                            Some((t.id, t.status, t.assigned_worker, t.base_commit.clone()))
                        } else {
                            None
                        }
                    })
                });
                if let Some((tid, status, worker, base_commit)) = ticket_info {
                    open_file_diff(app, tid, &status, worker, base_commit);
                }
                return InputResult::Continue;
            }
            KeyCode::Char('D') => {
                // Open clear confirm popup
                if let Some(ref tickets) = app.ticket_snapshot {
                    let has_completed = tickets.iter().any(|t| matches!(t.status, legion_core::orchestrate::engine::TicketStatus::Done | legion_core::orchestrate::engine::TicketStatus::Error));
                    if has_completed {
                        app.mode = AppMode::Popup(PopupMenu::ClearConfirm);
                    }
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

    // Shift+PageUp/Down: scroll focused pane (clamped to scrollback buffer size)
    if key.modifiers.contains(KeyModifiers::SHIFT) {
        match key.code {
            KeyCode::PageUp => {
                if let Some(pane) = app.panes.get_mut(app.focused_pane) {
                    pane.scroll_offset = pane.scroll_offset.saturating_add(10).min(crate::app::MAX_SCROLL_OFFSET);
                }
                return InputResult::Continue;
            }
            KeyCode::PageDown => {
                if let Some(pane) = app.panes.get_mut(app.focused_pane) {
                    pane.scroll_offset = pane.scroll_offset.saturating_sub(10);
                }
                return InputResult::Continue;
            }
            _ => {}
        }
    }

    // Any non-scroll key resets scroll offset (back to live view)
    if let Some(pane) = app.panes.get_mut(app.focused_pane) {
        if pane.scroll_offset > 0
            && !key.modifiers.contains(KeyModifiers::SHIFT)
        {
            pane.scroll_offset = 0;
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
    // Single mode: only handle scroll
    if !app.is_squad() {
        match event.kind {
            MouseEventKind::ScrollUp => {
                if let Some(pane) = app.panes.get_mut(app.focused_pane) {
                    pane.scroll_offset = pane.scroll_offset.saturating_add(3).min(crate::app::MAX_SCROLL_OFFSET);
                }
            }
            MouseEventKind::ScrollDown => {
                if let Some(pane) = app.panes.get_mut(app.focused_pane) {
                    pane.scroll_offset = pane.scroll_offset.saturating_sub(3);
                }
            }
            _ => {}
        }
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
            if matches!(app.mode, AppMode::Popup(PopupMenu::FileDiff)) {
                app.diff_scroll = app.diff_scroll.saturating_sub(3);
            } else if app.board_detail_open {
                app.board_detail_scroll = app.board_detail_scroll.saturating_sub(3);
            } else if app.right_panel_focused {
                navigate_ticket_up(app);
            } else if matches!(app.mode, AppMode::Normal) {
                // Scroll focused pane up (into scrollback)
                if let Some(pane) = app.panes.get_mut(app.focused_pane) {
                    pane.scroll_offset = pane.scroll_offset.saturating_add(3).min(crate::app::MAX_SCROLL_OFFSET);
                }
            }
        }
        MouseEventKind::ScrollDown => {
            if matches!(app.mode, AppMode::Popup(PopupMenu::FileDiff)) {
                app.diff_scroll = app.diff_scroll.saturating_add(3);
            } else if app.board_detail_open {
                app.board_detail_scroll = app.board_detail_scroll.saturating_add(3);
            } else if app.right_panel_focused {
                navigate_ticket_down(app);
            } else if matches!(app.mode, AppMode::Normal) {
                // Scroll focused pane down (towards live view)
                if let Some(pane) = app.panes.get_mut(app.focused_pane) {
                    pane.scroll_offset = pane.scroll_offset.saturating_sub(3);
                }
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
        AppMode::Popup(PopupMenu::ConnectProvider) => handle_connect_provider_keys(app, key),
        AppMode::Popup(PopupMenu::ProviderApiKeyInput) => handle_api_key_input_keys(app, key),
        AppMode::Popup(PopupMenu::MaxRetries) => handle_max_retries_keys(app, key),
        AppMode::Popup(PopupMenu::RetryForm) => handle_retry_form_keys(app, key),
        AppMode::Popup(PopupMenu::DeleteConfirm) => handle_delete_confirm_keys(app, key),
        AppMode::Popup(PopupMenu::ClearConfirm) => handle_clear_confirm_keys(app, key),
        AppMode::Popup(PopupMenu::SessionDeleteConfirm) => handle_session_delete_confirm_keys(app, key),
        AppMode::Popup(PopupMenu::CompleteRecordChoice) => handle_complete_record_choice_keys(app, key),
        AppMode::Popup(PopupMenu::FileDiff) => handle_file_diff_keys(app, key),
        AppMode::Popup(PopupMenu::BranchRecovery) => handle_branch_recovery_keys(app, key),
        AppMode::Popup(PopupMenu::BranchList) => handle_branch_list_keys(app, key),
        AppMode::Popup(PopupMenu::BranchChanged) => handle_branch_changed_keys(app, key),
        AppMode::Popup(PopupMenu::CopilotAuth) => handle_copilot_auth_keys(app, key),
        AppMode::Popup(PopupMenu::SetWorkerCount) => handle_set_worker_count_keys(app, key),
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
        KeyCode::Char('n') => {
            // Open new session input popup with branch selector
            app.session_name_input = app.default_session_name();
            init_new_session_branch_list(app);
            app.mode = AppMode::Popup(PopupMenu::NewSessionInput);
        }
        KeyCode::Char('d') => {
            // Delete selected session (only if active and NOT is_default)
            if app.session_list_index < app.session_list.len() {
                let session = &app.session_list[app.session_list_index];
                if session.status == "active" && !session.is_default {
                    let name = session.name.clone();
                    // Get session stats from DB
                    if let Ok(repo) = legion_db::open_db() {
                        app.session_delete_pending_count = repo.count_pending_tickets(&name).unwrap_or(0);
                        app.session_delete_ticket_count = repo.count_tickets(&name).unwrap_or(0);
                        app.session_delete_log_count = repo.count_ticket_logs(&name).unwrap_or(0);
                    }
                    app.session_delete_target = Some(name);
                    app.mode = AppMode::Popup(PopupMenu::SessionDeleteConfirm);
                }
            }
        }
        KeyCode::Char('c') => {
            // Complete selected session (only if active and NOT is_default)
            if app.session_list_index < app.session_list.len() {
                let session = &app.session_list[app.session_list_index];
                if session.status == "active" && !session.is_default {
                    app.complete_session_name = Some(session.name.clone());
                    app.complete_merge_index = 0;
                    app.mode = AppMode::Popup(PopupMenu::CompleteSession);
                }
            }
        }
        KeyCode::Char('x') => {
            // Remove completed session from history
            if app.session_list_index < app.session_list.len() {
                let session = &app.session_list[app.session_list_index];
                if session.status == "completed" {
                    let name = session.name.clone();
                    if let Ok(repo) = legion_db::open_db() {
                        if let Err(e) = repo.delete_squad_session(&name) {
                            tracing::error!("Failed to remove session '{}': {}", name, e);
                        } else {
                            tracing::info!("Removed completed session: {}", name);
                        }
                    }
                    app.load_session_list();
                    app.session_list_index = 0;
                }
            }
        }
        KeyCode::Enter => {
            if app.session_list_index >= app.session_list.len() {
                // "New Session" selected → show text input with branch selector
                app.session_name_input = app.default_session_name();
                init_new_session_branch_list(app);
                app.mode = AppMode::Popup(PopupMenu::NewSessionInput);
            } else {
                // Resume existing session
                let session = app.session_list[app.session_list_index].clone();

                // Check if branch was deleted
                if let (Some(ref branch), Some(ref project_path)) = (&session.base_branch, &app.project_path) {
                    if !crate::worktree::branch_exists(project_path, branch) {
                        // Detect current branch for recovery option display
                        app.detected_branch = crate::worktree::current_branch(project_path);
                        app.detected_commit = crate::worktree::current_commit(project_path);
                        app.recovery_session = Some(session);
                        app.recovery_choice = 0;
                        app.mode = AppMode::Popup(PopupMenu::BranchRecovery);
                        return;
                    }
                }

                // Normal resume (branch exists or no branch info)
                let workers = session.worker_count as u16;
                let is_default = session.is_default;
                match app.start_session(&session.name, workers, true, is_default) {
                    Ok(()) => {
                        tracing::info!("Resumed session: {}", session.name);
                        // Sync detected_branch/commit from the resumed session
                        if let Some(ref current) = app.current_session {
                            app.detected_branch = current.base_branch.clone();
                            app.detected_commit = current.base_commit.clone();
                        }
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
            app.complete_session_name = None;
            app.mode = AppMode::Popup(PopupMenu::SessionList);
        }
        KeyCode::Enter => {
            match app.complete_session_merge() {
                Ok(_) => {
                    app.complete_record_choice = 0;
                    app.mode = AppMode::Popup(PopupMenu::CompleteRecordChoice);
                }
                Err(e) => {
                    tracing::error!("Failed to merge session: {}", e);
                    app.complete_session_name = None;
                    app.mode = AppMode::Popup(PopupMenu::SessionList);
                }
            }
        }
        _ => {}
    }
}

fn handle_new_session_input_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            // Go back to session list
            app.session_name_input.clear();
            app.session_error = None;
            app.new_session_branch_focused = false;
            app.load_session_list();
            app.mode = AppMode::Popup(PopupMenu::SessionList);
            app.session_list_index = 0;
        }
        KeyCode::Tab | KeyCode::BackTab => {
            // Toggle focus between branch selector and name input
            app.new_session_branch_focused = !app.new_session_branch_focused;
        }
        KeyCode::Enter => {
            if app.new_session_branch_focused {
                // On branch: Enter confirms branch, switch focus to name
                app.new_session_branch_focused = false;
            } else {
                // On name: create session
                let name = app.session_name_input.trim().to_string();
                if !name.is_empty() {
                    // Set detected_branch to selected branch
                    if app.new_session_branch_index < app.branch_list.len() {
                        let branch = app.branch_list[app.new_session_branch_index].clone();
                        app.detected_branch = Some(branch);
                        if let Some(ref project_path) = app.project_path {
                            app.detected_commit = crate::worktree::current_commit(project_path);
                        }
                    }
                    let workers = app.requested_workers;
                    let is_default = app.creating_default_session;
                    match app.start_session(&name, workers, false, is_default) {
                        Ok(()) => {
                            tracing::info!("Created new session: {}", name);
                            app.session_name_input.clear();
                            app.creating_default_session = false;
                            app.new_session_branch_focused = false;
                            update_proxy_config(app);
                            app.mode = AppMode::Normal;
                        }
                        Err(e) => {
                            tracing::error!("Failed to create session '{}': {}", name, e);
                            app.session_error = Some(format!("{}", e));
                        }
                    }
                }
            }
        }
        KeyCode::Up | KeyCode::Char('k') if app.new_session_branch_focused => {
            if app.new_session_branch_index > 0 {
                app.new_session_branch_index -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') if app.new_session_branch_focused => {
            if app.new_session_branch_index < app.branch_list.len().saturating_sub(1) {
                app.new_session_branch_index += 1;
            }
        }
        KeyCode::Backspace if !app.new_session_branch_focused => {
            app.session_name_input.pop();
        }
        KeyCode::Char(c) if !app.new_session_branch_focused => {
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && (c.is_alphanumeric() || c == '-' || c == '_')
            {
                app.session_name_input.push(c);
            }
        }
        _ => {}
    }
}

/// Load branch list and pre-select detected_branch for new session popup
fn init_new_session_branch_list(app: &mut App) {
    if let Some(ref project_path) = app.project_path {
        app.branch_list = crate::worktree::list_local_branches(project_path);
    }
    app.new_session_branch_index = app.detected_branch.as_ref()
        .and_then(|db| app.branch_list.iter().position(|b| b == db))
        .unwrap_or(0);
    app.new_session_branch_focused = true; // start with branch selector focused
    app.session_error = None;
}

fn handle_remove_worker_list_keys(app: &mut App, key: KeyEvent) {
    let wc = app.worker_count();

    if app.remove_worker_confirming {
        // Stage 2: confirm action
        match key.code {
            KeyCode::Esc => {
                // Back to worker selection
                app.remove_worker_confirming = false;
            }
            KeyCode::Enter => {
                // Default: merge
                let pane_index = app.remove_worker_target + 1;
                app.pending_remove_worker = Some((pane_index, "merge".to_string()));
                app.remove_worker_confirming = false;
                app.mode = AppMode::Normal;
            }
            KeyCode::Char('k') | KeyCode::Char('K') => {
                let pane_index = app.remove_worker_target + 1;
                app.pending_remove_worker = Some((pane_index, "keep".to_string()));
                app.remove_worker_confirming = false;
                app.mode = AppMode::Normal;
            }
            KeyCode::Char('d') | KeyCode::Char('D') => {
                let pane_index = app.remove_worker_target + 1;
                app.pending_remove_worker = Some((pane_index, "discard".to_string()));
                app.remove_worker_confirming = false;
                app.mode = AppMode::Normal;
            }
            _ => {}
        }
    } else {
        // Stage 1: select worker
        match key.code {
            KeyCode::Esc => {
                app.mode = AppMode::Popup(PopupMenu::Main);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if app.remove_worker_target > 0 {
                    app.remove_worker_target -= 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if app.remove_worker_target + 1 < wc {
                    app.remove_worker_target += 1;
                }
            }
            KeyCode::Enter => {
                app.remove_worker_confirming = true;
            }
            _ => {}
        }
    }
}

fn handle_connect_provider_keys(app: &mut App, key: KeyEvent) {
    let template_count = crate::app::PROVIDER_TEMPLATES.len();
    match key.code {
        KeyCode::Esc => {
            app.mode = AppMode::Popup(PopupMenu::Main);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.connect_provider_index > 0 {
                app.connect_provider_index -= 1;
            } else {
                app.connect_provider_index = template_count.saturating_sub(1);
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.connect_provider_index = (app.connect_provider_index + 1) % template_count;
        }
        KeyCode::Enter => {
            let tmpl = &crate::app::PROVIDER_TEMPLATES[app.connect_provider_index];
            if tmpl.auth_method == "device_flow" {
                // Start Copilot device auth flow
                app.copilot_auth_status = crate::app::CopilotAuthStatus::RequestingCode;
                app.copilot_user_code = None;
                app.copilot_verification_uri = None;
                app.copilot_auth_error = None;
                app.copilot_models_result = None;

                // Spawn async auth task
                let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
                app.copilot_auth_rx = Some(rx);

                let provider_index = app.connect_provider_index;
                tokio::spawn(async move {
                    // Step 1: Request device code
                    match legion_core::copilot::request_device_code().await {
                        Ok(dc) => {
                            let _ = tx.send(crate::app::CopilotAuthMsg::DeviceCode {
                                user_code: dc.user_code,
                                verification_uri: dc.verification_uri,
                            });
                            // Step 2: Poll for access token
                            match legion_core::copilot::poll_for_access_token(&dc.device_code, dc.interval).await {
                                Ok(github_token) => {
                                    let _ = tx.send(crate::app::CopilotAuthMsg::Authorized);
                                    // Step 3: Full setup (exchange + fetch models)
                                    match legion_core::copilot::full_setup(&github_token).await {
                                        Ok((_info, models)) => {
                                            // Save provider to DB
                                            let tmpl = &crate::app::PROVIDER_TEMPLATES[provider_index];
                                            if let Ok(repo) = legion_db::open_db() {
                                                let now = std::time::SystemTime::now()
                                                    .duration_since(std::time::UNIX_EPOCH)
                                                    .unwrap_or_default()
                                                    .as_secs() as i64;
                                                let provider = legion_db::Provider {
                                                    id: tmpl.id.to_string(),
                                                    name: tmpl.name.to_string(),
                                                    base_url: tmpl.base_url.to_string(),
                                                    api_key: Some(github_token),
                                                    api_format: tmpl.api_format.to_string(),
                                                    models: Some(models.clone()),
                                                    is_default: false,
                                                    created_at: now,
                                                };
                                                let _ = repo.upsert_provider(&provider);
                                            }
                                            let _ = tx.send(crate::app::CopilotAuthMsg::SetupComplete { models });
                                        }
                                        Err(e) => {
                                            let _ = tx.send(crate::app::CopilotAuthMsg::Error(format!("Setup failed: {}", e)));
                                        }
                                    }
                                }
                                Err(e) => {
                                    let _ = tx.send(crate::app::CopilotAuthMsg::Error(format!("{}", e)));
                                }
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(crate::app::CopilotAuthMsg::Error(format!("{}", e)));
                        }
                    }
                });

                app.mode = AppMode::Popup(PopupMenu::CopilotAuth);
            } else {
                // Standard API key input
                let tmpl = &crate::app::PROVIDER_TEMPLATES[app.connect_provider_index];
                app.api_key_input = std::env::var(tmpl.env_var).unwrap_or_default();
                app.mode = AppMode::Popup(PopupMenu::ProviderApiKeyInput);
            }
        }
        _ => {}
    }
}

fn handle_api_key_input_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.api_key_input.clear();
            app.mode = AppMode::Popup(PopupMenu::ConnectProvider);
        }
        KeyCode::Enter => {
            // Save provider with API key
            let idx = app.connect_provider_index;
            let key_text = app.api_key_input.clone();
            app.save_provider_from_template(idx, &key_text);
            app.api_key_input.clear();
            app.mode = AppMode::Popup(PopupMenu::ConnectProvider);
        }
        KeyCode::Backspace => {
            app.api_key_input.pop();
        }
        KeyCode::Char(c) => {
            app.api_key_input.push(c);
        }
        _ => {}
    }
}

fn handle_max_retries_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.mode = AppMode::Popup(PopupMenu::Main);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.submenu_index > 0 {
                app.submenu_index -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.submenu_index < 9 {
                app.submenu_index += 1;
            }
        }
        KeyCode::Enter => {
            app.default_max_iterations = (app.submenu_index as u16) + 1;
            app.pending_sync_max_iterations = true;
            // Persist to DB
            if let Some(ref mut session) = app.current_session {
                session.max_iterations = Some(app.default_max_iterations as i64);
                if let Ok(repo) = legion_db::open_db() {
                    let _ = repo.upsert_squad_session(session);
                }
            }
            app.mode = AppMode::Popup(PopupMenu::Main);
            tracing::info!("Max retries set to {}", app.default_max_iterations);
        }
        _ => {}
    }
}

/// Build ticket IDs in the same visual order as the board renders them:
/// Working → Queued → Done → Error (matching ui.rs render_squad_board)
fn board_visual_order(tickets: &[legion_core::orchestrate::engine::TicketSnapshot]) -> Vec<usize> {
    use legion_core::orchestrate::engine::TicketStatus;
    let mut ids = Vec::new();
    for t in tickets.iter().filter(|t| t.status == TicketStatus::Working) {
        ids.push(t.id);
    }
    for t in tickets.iter().filter(|t| t.status == TicketStatus::Queued) {
        ids.push(t.id);
    }
    for t in tickets.iter().filter(|t| t.status == TicketStatus::Done) {
        ids.push(t.id);
    }
    for t in tickets.iter().filter(|t| t.status == TicketStatus::Error) {
        ids.push(t.id);
    }
    ids
}

fn navigate_ticket_down(app: &mut App) {
    if let Some(ref tickets) = app.ticket_snapshot {
        if tickets.is_empty() {
            return;
        }
        let ids = board_visual_order(tickets);
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
        let ids = board_visual_order(tickets);
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
            match client
                .post(format!("http://127.0.0.1:{}/legion/config", port))
                .json(&body)
                .send()
                .await
            {
                Ok(resp) => tracing::debug!("Proxy config sent to port {}: {}", port, resp.status()),
                Err(e) => tracing::error!("Failed to send proxy config to port {}: {}", port, e),
            }
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
fn handle_retry_form_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.mode = AppMode::Normal;
        }
        KeyCode::Tab => {
            app.retry_form_focus = (app.retry_form_focus + 1) % 4;
        }
        KeyCode::BackTab => {
            app.retry_form_focus = if app.retry_form_focus == 0 { 3 } else { app.retry_form_focus - 1 };
        }
        KeyCode::Enter => {
            // Submit retry
            if let Some(engine) = app.orchestrate.clone() {
                let tid = app.retry_target_id;
                let new_prompt = Some(app.retry_form_fields[0].clone());
                let new_context = Some(if app.retry_form_fields[1].is_empty() { None } else { Some(app.retry_form_fields[1].clone()) });
                let new_criteria = Some(if app.retry_form_fields[2].is_empty() { None } else { Some(app.retry_form_fields[2].clone()) });
                let feedback = if app.retry_form_fields[3].is_empty() { None } else { Some(app.retry_form_fields[3].clone()) };
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        if engine.retry_ticket(tid, new_prompt, new_context, new_criteria, feedback).await {
                            // Don't remove ticket_logs — preserve old logs, new ones will append
                            tracing::info!("Retrying ticket {} with updated fields", tid);
                        }
                    });
                });
            }
            app.mode = AppMode::Normal;
        }
        KeyCode::Backspace => {
            let idx = app.retry_form_focus as usize;
            app.retry_form_fields[idx].pop();
        }
        KeyCode::Char(c) => {
            let idx = app.retry_form_focus as usize;
            app.retry_form_fields[idx].push(c);
        }
        _ => {}
    }
}

fn handle_delete_confirm_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.mode = AppMode::Normal;
        }
        KeyCode::Enter | KeyCode::Char('y') => {
            if let Some(engine) = app.orchestrate.clone() {
                let tid = app.delete_confirm_id;
                tokio::task::block_in_place(|| {
                    tokio::runtime::Handle::current().block_on(async {
                        if engine.delete_ticket(tid).await {
                            app.ticket_logs.remove(&tid);
                            tracing::info!("Deleted ticket {}", tid);
                        }
                    });
                });
            }
            app.mode = AppMode::Normal;
        }
        _ => {}
    }
}

fn handle_session_delete_confirm_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter | KeyCode::Char('y') => {
            if let Some(name) = app.session_delete_target.clone() {
                match app.delete_session(&name) {
                    Ok(()) => tracing::info!("Deleted session: {}", name),
                    Err(e) => tracing::error!("Failed to delete session: {}", e),
                }
            }
            app.session_delete_target = None;
            app.load_session_list();
            app.session_list_index = 0;
            app.mode = AppMode::Popup(PopupMenu::SessionList);
        }
        KeyCode::Esc | KeyCode::Char('n') => {
            app.session_delete_target = None;
            app.mode = AppMode::Popup(PopupMenu::SessionList);
        }
        _ => {}
    }
}

fn handle_complete_record_choice_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            if app.complete_record_choice > 0 {
                app.complete_record_choice -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.complete_record_choice < 1 {
                app.complete_record_choice += 1;
            }
        }
        KeyCode::Enter => {
            let migrate = app.complete_record_choice == 1;
            match app.complete_session_records(migrate) {
                Ok(()) => tracing::info!("Session completed"),
                Err(e) => tracing::error!("Failed: {}", e),
            }
            app.complete_session_name = None;
            app.load_session_list();
            app.session_list_index = 0;
            app.mode = AppMode::Popup(PopupMenu::SessionList);
        }
        _ => {} // No Esc — must make a choice (merge already done)
    }
}

fn handle_clear_confirm_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.mode = AppMode::Normal;
        }
        KeyCode::Enter | KeyCode::Char('y') => {
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
            app.mode = AppMode::Normal;
        }
        _ => {}
    }
}

/// Open file diff popup for a ticket
fn open_file_diff(
    app: &mut App,
    ticket_id: usize,
    status: &legion_core::orchestrate::engine::TicketStatus,
    assigned_worker: Option<u16>,
    base_commit: Option<String>,
) {
    app.diff_ticket_id = ticket_id;
    app.diff_file_selected = 0;
    app.diff_scroll = 0;
    app.diff_loading = true;
    app.diff_error = None;
    app.diff_data = None;
    app.mode = AppMode::Popup(PopupMenu::FileDiff);

    // Try DB cache first for Done/Error
    if matches!(status,
        legion_core::orchestrate::engine::TicketStatus::Done
        | legion_core::orchestrate::engine::TicketStatus::Error
    ) {
        if let Some(engine) = &app.orchestrate {
            if let Some(db_arc) = engine.db() {
                if let Ok(db) = db_arc.lock() {
                    if let Ok(Some(cached)) = db.get_ticket_diff(ticket_id as i64) {
                        if !cached.diff_content.trim().is_empty() {
                            // Rebuild DiffData from cached
                            let files: Vec<_> = if cached.file_summary.is_empty() {
                                // file_summary was empty but raw diff has content — parse directly
                                crate::diff::parse_diff(&cached.diff_content)
                            } else {
                                let summary_files = cached.file_summary.iter().map(|fs| {
                                    crate::diff::DiffFile {
                                        path: fs.path.clone(),
                                        status: fs.status.clone(),
                                        additions: fs.additions,
                                        deletions: fs.deletions,
                                        diff_lines: Vec::new(),
                                    }
                                }).collect::<Vec<_>>();
                                let data = crate::diff::parse_raw_diff_with_summary(&cached.diff_content, summary_files);
                                data.files
                            };
                            app.diff_data = Some(crate::diff::DiffData {
                                files,
                                raw_diff: cached.diff_content,
                            });
                            app.diff_loading = false;
                            return;
                        }
                        // Cached diff was empty — fall through to git diff
                    }
                }
            }
        }
    }

    // Real-time fetch from git — always include uncommitted changes
    // Workers may forget to commit, so we need to capture untracked/unstaged files too
    let include_uncommitted = true;
    if let (Some(project_path), Some(session)) = (&app.project_path, &app.current_session) {
        if let Some(worker_id) = assigned_worker {
            let wt_path = crate::worktree::pane_worktree_path(
                project_path, &session.name, &format!("Worker {}", worker_id),
            );
            // Use base_commit for accurate diff if available
            let diff_result = if let Some(ref base) = base_commit {
                crate::diff::get_worktree_diff_from_commit(&wt_path, base)
            } else {
                let leader_ref = crate::diff::get_leader_ref(
                    project_path, &session.name, session.is_default,
                );
                crate::diff::get_worktree_diff(&wt_path, &leader_ref, include_uncommitted)
            };
            match diff_result {
                Ok(data) => {
                    app.diff_data = Some(data);
                    app.diff_loading = false;
                }
                Err(e) => {
                    app.diff_error = Some(format!("Failed to get diff: {}", e));
                    app.diff_loading = false;
                }
            }
        } else {
            app.diff_error = Some("No worker assigned".to_string());
            app.diff_loading = false;
        }
    } else {
        app.diff_error = Some("No active session".to_string());
        app.diff_loading = false;
    }
}

fn handle_file_diff_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.mode = AppMode::Normal;
            app.diff_data = None;
            app.diff_error = None;
        }
        KeyCode::Up => {
            // File list: move up
            if app.diff_file_selected > 0 {
                app.diff_file_selected -= 1;
                app.diff_scroll = 0;
            }
        }
        KeyCode::Down => {
            // File list: move down
            if let Some(ref data) = app.diff_data {
                if app.diff_file_selected < data.files.len().saturating_sub(1) {
                    app.diff_file_selected += 1;
                    app.diff_scroll = 0;
                }
            }
        }
        KeyCode::Char('j') => {
            app.diff_scroll = app.diff_scroll.saturating_add(1);
        }
        KeyCode::Char('k') => {
            app.diff_scroll = app.diff_scroll.saturating_sub(1);
        }
        KeyCode::PageDown => {
            app.diff_scroll = app.diff_scroll.saturating_add(20);
        }
        KeyCode::PageUp => {
            app.diff_scroll = app.diff_scroll.saturating_sub(20);
        }
        KeyCode::Home => {
            app.diff_scroll = 0;
        }
        KeyCode::End => {
            if let Some(ref data) = app.diff_data {
                if let Some(file) = data.files.get(app.diff_file_selected) {
                    app.diff_scroll = file.diff_lines.len().saturating_sub(1);
                }
            }
        }
        _ => {}
    }
}

fn update_session_branch(app: &App, session_name: &str) {
    if let Ok(repo) = legion_db::open_db() {
        if let Ok(Some(mut session)) = repo.get_squad_session(session_name) {
            session.base_branch = app.detected_branch.clone();
            session.base_commit = app.detected_commit.clone();
            let _ = repo.upsert_squad_session(&session);
        }
    }
}

fn resume_session(app: &mut App, session: &legion_db::SquadSession) {
    let workers = session.worker_count as u16;
    let is_default = session.is_default;
    match app.start_session(&session.name, workers, true, is_default) {
        Ok(()) => {
            tracing::info!("Resumed session: {}", session.name);
            // Sync detected_branch/commit from the session being resumed
            if let Some(ref current) = app.current_session {
                app.detected_branch = current.base_branch.clone();
                app.detected_commit = current.base_commit.clone();
            }
            update_proxy_config(app);
            app.mode = AppMode::Normal;
        }
        Err(e) => {
            tracing::error!("Failed to resume session '{}': {}", session.name, e);
            app.mode = AppMode::Popup(PopupMenu::SessionList);
        }
    }
}

fn handle_branch_recovery_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.recovery_session = None;
            app.load_session_list();
            app.mode = AppMode::Popup(PopupMenu::SessionList);
            app.session_list_index = 0;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.recovery_choice > 0 { app.recovery_choice -= 1; }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.recovery_choice < 3 { app.recovery_choice += 1; }
        }
        KeyCode::Enter => {
            let session = match app.recovery_session.take() {
                Some(s) => s,
                None => return,
            };
            match app.recovery_choice {
                0 => {
                    // Bind to current branch and continue
                    update_session_branch(app, &session.name);
                    resume_session(app, &session);
                }
                1 => {
                    // Select another branch
                    if let Some(ref project_path) = app.project_path {
                        app.branch_list = crate::worktree::list_local_branches(project_path);
                    }
                    app.branch_list_index = 0;
                    app.recovery_session = Some(session);
                    app.mode = AppMode::Popup(PopupMenu::BranchList);
                }
                2 => {
                    // Create new branch from base commit
                    if let (Some(ref commit), Some(ref project_path)) = (&session.base_commit, &app.project_path) {
                        let branch_name = session.base_branch.as_deref().unwrap_or("recovered");
                        let _ = std::process::Command::new("git")
                            .args(["branch", branch_name, commit])
                            .current_dir(project_path)
                            .output();
                        app.detected_branch = Some(branch_name.to_string());
                        app.detected_commit = Some(commit.clone());
                    }
                    update_session_branch(app, &session.name);
                    resume_session(app, &session);
                }
                3 | _ => {
                    // Cancel
                    app.load_session_list();
                    app.mode = AppMode::Popup(PopupMenu::SessionList);
                    app.session_list_index = 0;
                }
            }
        }
        _ => {}
    }
}

fn handle_branch_list_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            if app.recovery_session.is_some() {
                // Back to recovery dialog
                app.recovery_choice = 1;
                app.mode = AppMode::Popup(PopupMenu::BranchRecovery);
            } else {
                // Back to main menu (manual switch)
                app.mode = AppMode::Popup(PopupMenu::Main);
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.branch_list_index > 0 { app.branch_list_index -= 1; }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.branch_list_index < app.branch_list.len().saturating_sub(1) {
                app.branch_list_index += 1;
            }
        }
        KeyCode::Enter => {
            if app.branch_list_index < app.branch_list.len() {
                let branch = app.branch_list[app.branch_list_index].clone();
                app.detected_branch = Some(branch.clone());
                if let Some(ref project_path) = app.project_path {
                    app.detected_commit = crate::worktree::current_commit(project_path);
                }
                if let Some(session) = app.recovery_session.take() {
                    // Recovery flow — update and resume
                    update_session_branch(app, &session.name);
                    resume_session(app, &session);
                } else {
                    // Manual switch — update current session, show rebuild/rebase prompt
                    if let Some(ref session) = app.current_session {
                        update_session_branch(app, &session.name);
                    }
                    app.branch_changed_to = Some(branch);
                    app.recovery_choice = 0;
                    app.mode = AppMode::Popup(PopupMenu::BranchChanged);
                }
            }
        }
        _ => {}
    }
}

fn handle_branch_changed_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            // Ignore — keep current binding
            app.branch_changed_to = None;
            app.mode = AppMode::Normal;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if app.recovery_choice > 0 { app.recovery_choice -= 1; }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.recovery_choice < 2 { app.recovery_choice += 1; }
        }
        KeyCode::Enter => {
            let new_branch = match app.branch_changed_to.take() {
                Some(b) => b,
                None => { app.mode = AppMode::Normal; return; }
            };
            match app.recovery_choice {
                0 => {
                    // Switch session — rebuild worktrees
                    app.detected_branch = Some(new_branch);
                    if let Some(ref project_path) = app.project_path {
                        app.detected_commit = crate::worktree::current_commit(project_path);
                    }
                    if let Some(ref session) = app.current_session {
                        update_session_branch(app, &session.name);
                    }
                    // Update in-memory session too
                    if let Some(ref mut session) = app.current_session {
                        session.base_branch = app.detected_branch.clone();
                        session.base_commit = app.detected_commit.clone();
                    }
                    app.mode = AppMode::Normal;
                }
                1 => {
                    // Switch session — rebase worktrees
                    app.detected_branch = Some(new_branch);
                    if let Some(ref project_path) = app.project_path {
                        app.detected_commit = crate::worktree::current_commit(project_path);
                    }
                    if let Some(ref session) = app.current_session {
                        update_session_branch(app, &session.name);
                    }
                    if let Some(ref mut session) = app.current_session {
                        session.base_branch = app.detected_branch.clone();
                        session.base_commit = app.detected_commit.clone();
                    }
                    app.mode = AppMode::Normal;
                }
                2 | _ => {
                    // Ignore
                    app.mode = AppMode::Normal;
                }
            }
        }
        _ => {}
    }
}

fn handle_copilot_auth_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            // Cancel — drop the channel receiver to signal the spawned task
            app.copilot_auth_rx = None;
            app.copilot_auth_status = crate::app::CopilotAuthStatus::RequestingCode;
            app.mode = AppMode::Popup(PopupMenu::ConnectProvider);
        }
        KeyCode::Enter => {
            match app.copilot_auth_status {
                crate::app::CopilotAuthStatus::Success => {
                    // Done — go back to connect provider list
                    app.copilot_auth_rx = None;
                    app.mode = AppMode::Popup(PopupMenu::ConnectProvider);
                }
                crate::app::CopilotAuthStatus::Error => {
                    // Retry — go back to connect provider, user can press Enter again
                    app.copilot_auth_rx = None;
                    app.copilot_auth_status = crate::app::CopilotAuthStatus::RequestingCode;
                    app.mode = AppMode::Popup(PopupMenu::ConnectProvider);
                }
                _ => {}
            }
        }
        _ => {}
    }
}

fn handle_set_worker_count_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            if app.set_worker_count_selection > 1 {
                app.set_worker_count_selection -= 1;
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if app.set_worker_count_selection < crate::app::MAX_WORKERS {
                app.set_worker_count_selection += 1;
            }
        }
        KeyCode::Char(c) if c.is_ascii_digit() => {
            let n = c.to_digit(10).unwrap() as u16;
            if n >= 1 && n <= crate::app::MAX_WORKERS {
                app.set_worker_count_selection = n;
            }
        }
        KeyCode::Enter => {
            let target = app.set_worker_count_selection;
            let current = app.worker_count() as u16;
            if target != current {
                app.pending_set_worker_count = Some(target);
            }
            app.mode = AppMode::Normal;
        }
        KeyCode::Esc => {
            app.mode = AppMode::Popup(PopupMenu::Main);
        }
        _ => {}
    }
}

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
