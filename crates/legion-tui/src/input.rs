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

    // Ctrl+T toggles dashboard overlay (squad only)
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('t') {
        if app.is_squad() {
            app.show_dashboard = !app.show_dashboard;
            return InputResult::Continue;
        }
    }

    // Squad-only shortcuts (not forwarded to PTY)
    if app.is_squad() {
        match key.code {
            // Tab / BackTab cycle pane focus
            KeyCode::Tab => {
                app.focus_next();
                return InputResult::Continue;
            }
            KeyCode::BackTab => {
                app.focus_prev();
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
        _ => {}
    }
}

fn handle_popup_mode(app: &mut App, key: KeyEvent) -> InputResult {
    // Ctrl+P also closes popup
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
        app.toggle_popup();
        return InputResult::Continue;
    }

    match app.mode {
        AppMode::Popup(PopupMenu::Matrix) => handle_matrix_keys(app, key),
        AppMode::Popup(PopupMenu::Main) => handle_main_menu_keys(app, key),
        AppMode::Popup(PopupMenu::Provider) | AppMode::Popup(PopupMenu::Model) => {
            handle_submenu_keys(app, key)
        }
        _ => {}
    }

    InputResult::Continue
}

fn handle_matrix_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => app.back_to_main_menu(),
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
            if app.model_target.is_some() {
                app.back_to_matrix();
            } else {
                app.back_to_main_menu();
            }
        }
        KeyCode::Up | KeyCode::Char('k') => app.menu_up(),
        KeyCode::Down | KeyCode::Char('j') => app.menu_down(),
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            app.select_submenu_item();
            update_proxy_config(app);
        }
        _ => {}
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
