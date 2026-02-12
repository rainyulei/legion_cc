//! TUI input handling

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, AppMode, PopupMenu};

/// Result of handling input
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputResult {
    /// Continue running
    Continue,
    /// Quit the application
    Quit,
}

/// Main key handler
pub fn handle_key(app: &mut App, key: KeyEvent) -> InputResult {
    // Global shortcuts (work in any mode)
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Char('q') => return InputResult::Quit,
            KeyCode::Char('p') => {
                app.toggle_popup();
                return InputResult::Continue;
            }
            _ => {}
        }
    }

    // Mode-specific handling
    match app.mode {
        AppMode::Normal => handle_normal_mode(app, key),
        AppMode::Popup(_) => handle_popup_mode(app, key),
    }
}

/// Handle input in normal mode (forward keys to PTY)
fn handle_normal_mode(app: &mut App, key: KeyEvent) -> InputResult {
    // Forward keys to PTY
    let bytes = key_to_bytes(key);
    if !bytes.is_empty() {
        app.send_to_pty(&bytes);
    }
    InputResult::Continue
}

/// Convert a key event to bytes for PTY
fn key_to_bytes(key: KeyEvent) -> Vec<u8> {
    match key.code {
        KeyCode::Char(c) => {
            if key.modifiers.contains(KeyModifiers::CONTROL) {
                vec![(c as u8) & 0x1f]
            } else {
                c.to_string().into_bytes()
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![0x7f],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => vec![0x1b, b'[', b'A'],
        KeyCode::Down => vec![0x1b, b'[', b'B'],
        KeyCode::Right => vec![0x1b, b'[', b'C'],
        KeyCode::Left => vec![0x1b, b'[', b'D'],
        _ => vec![],
    }
}

/// Handle input in popup mode
fn handle_popup_mode(app: &mut App, key: KeyEvent) -> InputResult {
    match key.code {
        KeyCode::Esc => {
            // ESC closes popup or goes back
            match app.mode {
                AppMode::Popup(PopupMenu::Main) => {
                    app.mode = AppMode::Normal;
                }
                AppMode::Popup(_) => {
                    app.back_to_main_menu();
                }
                _ => {}
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.menu_up();
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.menu_down();
        }
        KeyCode::Enter => {
            match app.mode {
                AppMode::Popup(PopupMenu::Main) => {
                    app.enter_submenu();
                    // Check if we should quit after entering submenu
                    if app.should_quit {
                        return InputResult::Quit;
                    }
                }
                AppMode::Popup(_) => {
                    app.select_submenu_item();
                }
                _ => {}
            }
        }
        KeyCode::Left | KeyCode::Char('h') => {
            // Go back in submenu
            if !matches!(app.mode, AppMode::Popup(PopupMenu::Main)) {
                app.back_to_main_menu();
            }
        }
        KeyCode::Right | KeyCode::Char('l') => {
            // Enter submenu (same as Enter for main menu)
            if matches!(app.mode, AppMode::Popup(PopupMenu::Main)) {
                app.enter_submenu();
                if app.should_quit {
                    return InputResult::Quit;
                }
            }
        }
        _ => {}
    }

    InputResult::Continue
}
