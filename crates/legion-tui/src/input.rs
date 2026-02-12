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

/// Handle input in normal mode (PTY passthrough placeholder)
fn handle_normal_mode(_app: &mut App, _key: KeyEvent) -> InputResult {
    // In normal mode, keys would be forwarded to the PTY
    // For now, this is a placeholder
    // TODO: Forward keys to PTY when integrated
    InputResult::Continue
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
