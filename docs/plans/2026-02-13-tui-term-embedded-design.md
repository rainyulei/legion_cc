# tui-term Embedded TUI Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Restore Legion's original ratatui TUI layout (header + bordered main + footer + popup overlay) with Claude Code properly rendered via tui-term's PseudoTerminal widget instead of raw Paragraph.

**Architecture:** Single-process ratatui app. Claude Code spawns in a PTY managed by portable-pty. PTY output is parsed by vt100 into a screen buffer, then rendered by tui-term's PseudoTerminal widget. Keyboard input routes to PTY in normal mode, or to popup menu navigation in menu mode. Proxy + control API run as background tokio tasks within the same process.

**Tech Stack:** ratatui 0.29, crossterm 0.28, tui-term 0.2, vt100 0.15, portable-pty 0.8, tokio, reqwest

---

### Task 1: Add dependencies

**Files:**
- Modify: `Cargo.toml` (workspace root, line 10)
- Modify: `crates/legion-tui/Cargo.toml`

**Step 1: Update workspace Cargo.toml**

Add to `[workspace.dependencies]` section:

```toml
portable-pty = "0.8"
tui-term = "0.2"
vt100 = "0.15"
```

And add `proto` to workspace exclude:

```toml
exclude = ["proto"]
```

**Step 2: Update legion-tui/Cargo.toml**

Add these dependencies:

```toml
portable-pty.workspace = true
tui-term.workspace = true
vt100.workspace = true
```

**Step 3: Verify it compiles**

Run: `cargo build -p legion-tui`
Expected: compiles with no errors

**Step 4: Commit**

```bash
git add Cargo.toml crates/legion-tui/Cargo.toml
git commit -m "deps: add tui-term, vt100, portable-pty for embedded terminal"
```

---

### Task 2: Create PTY module

Port the working prototype's PTY management into a reusable module.

**Files:**
- Create: `crates/legion-tui/src/pty.rs`

**Step 1: Write pty.rs**

```rust
//! PTY management for embedding Claude Code in ratatui
//!
//! Spawns Claude Code in a pseudo-terminal, reads output into a vt100 parser,
//! and provides a writer for sending keyboard input.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};

/// Shared parser state - accessed by reader thread and render loop
pub type SharedParser = Arc<Mutex<vt100::Parser>>;

/// Manages a PTY running Claude Code
pub struct PtyHandle {
    pub parser: SharedParser,
    writer: Box<dyn Write + Send>,
    _child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl PtyHandle {
    /// Spawn Claude Code in a PTY with the given size and environment
    pub fn spawn(
        rows: u16,
        cols: u16,
        proxy_port: u16,
        control_port: u16,
    ) -> Result<Self> {
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to open PTY")?;

        let mut cmd = CommandBuilder::new("claude");
        cmd.env(
            "ANTHROPIC_BASE_URL",
            format!("http://127.0.0.1:{}", proxy_port),
        );
        cmd.env("LEGION_CONTROL_PORT", control_port.to_string());

        let child = pair
            .slave
            .spawn_command(cmd)
            .context("Failed to spawn claude in PTY")?;
        drop(pair.slave);

        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 1000)));

        // Reader thread: PTY stdout -> vt100 parser
        let reader = pair
            .master
            .try_clone_reader()
            .context("Failed to clone PTY reader")?;
        let parser_clone = Arc::clone(&parser);
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut p) = parser_clone.lock() {
                            p.process(&buf[..n]);
                        }
                    }
                }
            }
        });

        let writer = pair
            .master
            .take_writer()
            .context("Failed to take PTY writer")?;

        Ok(Self {
            parser,
            writer,
            _child: child,
        })
    }

    /// Send bytes to the PTY (keyboard input)
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.writer
            .write_all(data)
            .context("Failed to write to PTY")?;
        self.writer.flush().ok();
        Ok(())
    }

    /// Resize the PTY and vt100 parser
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        if let Ok(mut p) = self.parser.lock() {
            p.set_size(rows, cols);
        }
        Ok(())
    }
}
```

**Step 2: Add module declaration**

In `crates/legion-tui/src/lib.rs`, add at top:
```rust
pub mod pty;
```

**Step 3: Verify it compiles**

Run: `cargo build -p legion-tui`
Expected: compiles (PtyHandle unused warning is fine)

**Step 4: Commit**

```bash
git add crates/legion-tui/src/pty.rs crates/legion-tui/src/lib.rs
git commit -m "feat(tui): add PTY module with vt100 parser for embedded terminal"
```

---

### Task 3: Refactor App state

Convert from popup-only state to full TUI with PTY and mode management.

**Files:**
- Rewrite: `crates/legion-tui/src/app.rs`

**Step 1: Rewrite app.rs**

```rust
//! TUI application state

use legion_db::Provider;

use crate::pty::{PtyHandle, SharedParser};

/// Application mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    /// Normal mode - keys go to PTY
    Normal,
    /// Popup menu mode - keys navigate menu
    Popup(PopupMenu),
}

/// Popup menu types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupMenu {
    Main,
    Provider,
    Model,
}

/// Main menu items
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainMenuItem {
    Provider,
    Model,
    Quit,
}

impl MainMenuItem {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Provider => "Provider",
            Self::Model => "Model",
            Self::Quit => "Quit",
        }
    }
}

/// Full TUI application state
pub struct App {
    pub mode: AppMode,
    pub should_quit: bool,

    // Provider/model state
    pub providers: Vec<Provider>,
    pub current_provider: Option<usize>,
    pub current_model: Option<String>,
    pub provider_connected: bool,

    // Menu navigation
    pub menu_index: usize,
    pub submenu_index: usize,

    // PTY (None until started)
    pub pty: Option<PtyHandle>,

    // Ports
    pub proxy_port: u16,
    pub control_port: u16,
}

impl App {
    pub fn new(proxy_port: u16, control_port: u16) -> Self {
        Self {
            mode: AppMode::Normal,
            should_quit: false,
            providers: Vec::new(),
            current_provider: None,
            current_model: None,
            provider_connected: false,
            menu_index: 0,
            submenu_index: 0,
            pty: None,
            proxy_port,
            control_port,
        }
    }

    /// Get shared parser ref for rendering (if PTY is running)
    pub fn parser(&self) -> Option<&SharedParser> {
        self.pty.as_ref().map(|p| &p.parser)
    }

    /// Send bytes to PTY
    pub fn write_to_pty(&mut self, data: &[u8]) {
        if let Some(ref mut pty) = self.pty {
            let _ = pty.write(data);
        }
    }

    /// Start Claude Code in PTY
    pub fn start_claude(&mut self, rows: u16, cols: u16) {
        match PtyHandle::spawn(rows, cols, self.proxy_port, self.control_port) {
            Ok(handle) => {
                self.pty = Some(handle);
            }
            Err(e) => {
                tracing::error!("Failed to spawn Claude: {}", e);
            }
        }
    }

    /// Load providers from database
    pub fn load_from_db(&mut self) {
        if let Ok(repo) = legion_db::open_db() {
            if let Ok(providers) = repo.list_providers() {
                self.providers = providers;
                if let Ok(Some(default)) = repo.get_default_provider() {
                    self.current_provider =
                        self.providers.iter().position(|p| p.id == default.id);
                    self.current_model =
                        default.models.as_ref().and_then(|m| m.first().cloned());
                    self.provider_connected = true;
                }
            }
        }
    }

    // ─── Menu navigation (unchanged from original) ───

    pub fn main_menu_items() -> &'static [MainMenuItem] {
        &[MainMenuItem::Provider, MainMenuItem::Model, MainMenuItem::Quit]
    }

    pub fn toggle_popup(&mut self) {
        match self.mode {
            AppMode::Normal => {
                self.mode = AppMode::Popup(PopupMenu::Main);
                self.menu_index = 0;
            }
            AppMode::Popup(_) => {
                self.mode = AppMode::Normal;
            }
        }
    }

    pub fn enter_submenu(&mut self) {
        if let AppMode::Popup(PopupMenu::Main) = self.mode {
            let items = Self::main_menu_items();
            if self.menu_index < items.len() {
                match items[self.menu_index] {
                    MainMenuItem::Provider => {
                        self.mode = AppMode::Popup(PopupMenu::Provider);
                        self.submenu_index = self.current_provider.unwrap_or(0);
                    }
                    MainMenuItem::Model => {
                        self.mode = AppMode::Popup(PopupMenu::Model);
                        self.submenu_index = self.get_current_model_index().unwrap_or(0);
                    }
                    MainMenuItem::Quit => {
                        self.should_quit = true;
                    }
                }
            }
        }
    }

    pub fn select_submenu_item(&mut self) {
        match self.mode {
            AppMode::Popup(PopupMenu::Provider) => {
                if self.submenu_index < self.providers.len() {
                    self.current_provider = Some(self.submenu_index);
                    self.current_model = self
                        .get_current_provider()
                        .and_then(|p| p.models.as_ref())
                        .and_then(|m| m.first().cloned());
                    self.provider_connected = true;
                }
                self.mode = AppMode::Popup(PopupMenu::Main);
            }
            AppMode::Popup(PopupMenu::Model) => {
                if let Some(models) = self.get_current_provider_models() {
                    if self.submenu_index < models.len() {
                        self.current_model = Some(models[self.submenu_index].clone());
                    }
                }
                self.mode = AppMode::Popup(PopupMenu::Main);
            }
            _ => {}
        }
    }

    pub fn back_to_main_menu(&mut self) {
        self.mode = AppMode::Popup(PopupMenu::Main);
    }

    pub fn menu_up(&mut self) {
        let len = match self.mode {
            AppMode::Popup(PopupMenu::Main) => Self::main_menu_items().len(),
            AppMode::Popup(PopupMenu::Provider) => self.providers.len(),
            AppMode::Popup(PopupMenu::Model) => {
                self.get_current_provider_models().map(|m| m.len()).unwrap_or(0)
            }
            _ => return,
        };
        let idx = match self.mode {
            AppMode::Popup(PopupMenu::Main) => &mut self.menu_index,
            _ => &mut self.submenu_index,
        };
        *idx = if *idx > 0 { *idx - 1 } else { len.saturating_sub(1) };
    }

    pub fn menu_down(&mut self) {
        let len = match self.mode {
            AppMode::Popup(PopupMenu::Main) => Self::main_menu_items().len(),
            AppMode::Popup(PopupMenu::Provider) => self.providers.len(),
            AppMode::Popup(PopupMenu::Model) => {
                self.get_current_provider_models().map(|m| m.len()).unwrap_or(0)
            }
            _ => return,
        };
        let idx = match self.mode {
            AppMode::Popup(PopupMenu::Main) => &mut self.menu_index,
            _ => &mut self.submenu_index,
        };
        *idx = if *idx < len.saturating_sub(1) { *idx + 1 } else { 0 };
    }

    pub fn get_current_provider(&self) -> Option<&Provider> {
        self.current_provider.and_then(|i| self.providers.get(i))
    }

    pub fn get_current_provider_models(&self) -> Option<&Vec<String>> {
        self.get_current_provider().and_then(|p| p.models.as_ref())
    }

    fn get_current_model_index(&self) -> Option<usize> {
        let models = self.get_current_provider_models()?;
        let current = self.current_model.as_ref()?;
        models.iter().position(|m| m == current)
    }
}
```

**Step 2: Verify it compiles**

Run: `cargo build -p legion-tui`
Expected: compiles (will have warnings about unused imports from other modules)

**Step 3: Commit**

```bash
git add crates/legion-tui/src/app.rs
git commit -m "refactor(tui): convert App to full TUI state with PTY and mode management"
```

---

### Task 4: Rewrite UI rendering

Restore original 3-part layout with PseudoTerminal widget.

**Files:**
- Rewrite: `crates/legion-tui/src/ui.rs`

**Step 1: Write ui.rs**

```rust
//! TUI rendering - header + bordered PTY area + footer + popup overlay

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};
use tui_term::widget::PseudoTerminal;

use crate::app::{App, AppMode, MainMenuItem, PopupMenu};

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
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let provider_name = app
        .get_current_provider()
        .map(|p| p.name.as_str())
        .unwrap_or("No Provider");
    let model_name = app.current_model.as_deref().unwrap_or("No Model");

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
    let block = Block::default()
        .title(" Claude Code ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));

    if let Some(parser) = app.parser() {
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
    let mode_hint = match app.mode {
        AppMode::Normal => vec![
            Span::styled(" Ctrl+P", Style::default().fg(Color::Yellow)),
            Span::styled(": Menu ", Style::default().fg(Color::DarkGray)),
            Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
            Span::styled("Ctrl+Q", Style::default().fg(Color::Yellow)),
            Span::styled(": Quit", Style::default().fg(Color::DarkGray)),
        ],
        AppMode::Popup(_) => vec![
            Span::styled(" j/k", Style::default().fg(Color::Yellow)),
            Span::styled(": Navigate ", Style::default().fg(Color::DarkGray)),
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::styled(": Select ", Style::default().fg(Color::DarkGray)),
            Span::styled("Esc", Style::default().fg(Color::Yellow)),
            Span::styled(": Close", Style::default().fg(Color::DarkGray)),
        ],
    };

    frame.render_widget(Paragraph::new(Line::from(mode_hint)), area);
}

// ─── Popup overlay ────────────────────────────────────────────────────────────

fn draw_popup(frame: &mut Frame, app: &App, menu: PopupMenu) {
    let area = centered_rect(50, 60, frame.area());
    frame.render_widget(Clear, area);

    match menu {
        PopupMenu::Main => draw_main_menu(frame, app, area),
        PopupMenu::Provider => draw_provider_menu(frame, app, area),
        PopupMenu::Model => draw_model_menu(frame, app, area),
    }
}

fn draw_main_menu(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Legion [ESC] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let items: Vec<ListItem> = App::main_menu_items()
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let selected = i == app.menu_index;
            let prefix = if selected { "> " } else { "  " };
            let value = match item {
                MainMenuItem::Provider => {
                    let name = app
                        .get_current_provider()
                        .map(|p| p.name.clone())
                        .unwrap_or_else(|| "None".into());
                    let dot = if app.provider_connected { " \u{25cf}" } else { "" };
                    format!("[{}{}]", name, dot)
                }
                MainMenuItem::Model => {
                    format!("[{}]", app.current_model.as_deref().unwrap_or("None"))
                }
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

    // Separator before Quit
    let mut final_items = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        if i == 2 {
            final_items.push(ListItem::new(Line::from(Span::styled(
                "  \u{2500}".repeat(12),
                Style::default().fg(Color::DarkGray),
            ))));
        }
        final_items.push(item);
    }

    frame.render_widget(List::new(final_items).block(block), area);
}

fn draw_provider_menu(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Select Provider [ESC] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

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
            let current = app.current_provider == Some(i);
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
        .title(" Select Model [ESC] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    if let Some(models) = app.get_current_provider_models() {
        let items: Vec<ListItem> = models
            .iter()
            .enumerate()
            .map(|(i, model)| {
                let selected = i == app.submenu_index;
                let current = app.current_model.as_deref() == Some(model);
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
```

**Step 2: Verify it compiles**

Run: `cargo build -p legion-tui`
Expected: compiles

**Step 3: Commit**

```bash
git add crates/legion-tui/src/ui.rs
git commit -m "feat(tui): restore original layout with PseudoTerminal widget"
```

---

### Task 5: Rewrite input handling

Mode-aware: forward to PTY in normal mode, navigate menu in popup mode.

**Files:**
- Rewrite: `crates/legion-tui/src/input.rs`

**Step 1: Write input.rs**

```rust
//! Input handling - mode-aware key routing

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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

    // Everything else goes to PTY
    let bytes = key_to_bytes(key);
    if !bytes.is_empty() {
        app.write_to_pty(&bytes);
    }

    InputResult::Continue
}

fn handle_popup_mode(app: &mut App, key: KeyEvent) -> InputResult {
    match key.code {
        KeyCode::Esc => match app.mode {
            AppMode::Popup(PopupMenu::Main) => app.toggle_popup(),
            _ => app.back_to_main_menu(),
        },
        KeyCode::Up | KeyCode::Char('k') => app.menu_up(),
        KeyCode::Down | KeyCode::Char('j') => app.menu_down(),
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            if let AppMode::Popup(PopupMenu::Main) = app.mode {
                app.enter_submenu();
            } else {
                app.select_submenu_item();
                // After selecting, update proxy config
                update_proxy_config(app);
            }
        }
        KeyCode::Left | KeyCode::Char('h') => {
            if app.mode != AppMode::Popup(PopupMenu::Main) {
                app.back_to_main_menu();
            }
        }
        _ => {}
    }

    InputResult::Continue
}

/// After provider/model selection, POST new config to control API
fn update_proxy_config(app: &App) {
    let provider = match app.get_current_provider() {
        Some(p) => p,
        None => return,
    };

    let base_url = provider.base_url.clone();
    let api_format = provider.api_format.clone();
    let api_key = provider.api_key.clone();
    let model = app.current_model.clone();
    let port = app.control_port;

    // Fire and forget - update in background
    tokio::spawn(async move {
        let client = reqwest::Client::new();
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
```

**Step 2: Verify it compiles**

Run: `cargo build -p legion-tui`

**Step 3: Commit**

```bash
git add crates/legion-tui/src/input.rs
git commit -m "feat(tui): mode-aware input routing with PTY key forwarding"
```

---

### Task 6: Rewrite lib.rs entry point

Single `run()` entry that starts proxy, control API, spawns Claude in PTY, and runs the TUI event loop.

**Files:**
- Rewrite: `crates/legion-tui/src/lib.rs`
- Delete: `crates/legion-tui/src/sidebar.rs` (no longer needed)

**Step 1: Write lib.rs**

```rust
//! Legion TUI - Embedded terminal interface with provider/model switching

pub mod app;
pub mod input;
pub mod pty;
pub mod ui;

use std::io;

use anyhow::Result;
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use app::App;
use input::{handle_key, InputResult};
use ui::draw;

/// Run the full TUI with embedded Claude Code
pub async fn run(proxy_port: u16, control_port: u16) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and load providers from DB
    let mut app = App::new(proxy_port, control_port);
    app.load_from_db();

    // Calculate PTY size from terminal (minus header=1, footer=1, border=2)
    let size = terminal.size()?;
    let pty_rows = size.height.saturating_sub(4);
    let pty_cols = size.width.saturating_sub(2);

    // Start Claude in PTY
    app.start_claude(pty_rows, pty_cols);

    // Main event loop
    let result = run_event_loop(&mut terminal, &mut app).await;

    // Restore terminal
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    result
}

/// Run the popup TUI only (for backward compat with `legion switch`)
pub async fn run_popup(control_port: u16) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(0, control_port);
    app.load_from_db();
    app.toggle_popup(); // Start in popup mode

    let result = run_event_loop(&mut terminal, &mut app).await;

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    result
}

async fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|frame| draw(frame, app))?;

        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match handle_key(app, key) {
                    InputResult::Quit => break,
                    InputResult::Continue => {}
                }
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}
```

**Step 2: Delete sidebar.rs**

Remove `crates/legion-tui/src/sidebar.rs`.

**Step 3: Verify it compiles**

Run: `cargo build -p legion-tui`

**Step 4: Commit**

```bash
git rm crates/legion-tui/src/sidebar.rs
git add crates/legion-tui/src/lib.rs
git commit -m "feat(tui): single run() entry with embedded PTY event loop"
```

---

### Task 7: Simplify CLI

Remove tmux sidebar/popup commands. The TUI runs directly.

**Files:**
- Rewrite: `crates/legion-cli/src/main.rs`

**Step 1: Write main.rs**

```rust
use anyhow::Result;
use clap::{Parser, Subcommand};
use legion_core::proxy::{ProxyControlApi, ProxyServer};

#[derive(Parser)]
#[command(name = "legion")]
#[command(about = "Claude Code companion - provider/model switching")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start legion TUI with embedded Claude Code
    Start {
        /// Proxy port
        #[arg(long, default_value = "18080")]
        port: u16,

        /// Control API port
        #[arg(long, default_value = "19080")]
        control_port: u16,

        /// Only start servers, no TUI (for testing)
        #[arg(long)]
        serve_only: bool,
    },

    /// Interactive provider/model switch (standalone popup)
    Switch {
        #[arg(long, default_value = "19080")]
        control_port: u16,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("legion=info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Start {
            port,
            control_port,
            serve_only,
        }) => {
            cmd_start(port, control_port, serve_only).await?;
        }
        Some(Commands::Switch { control_port }) => {
            legion_tui::run_popup(control_port).await?;
        }
        None => {
            cmd_start(18080, 19080, false).await?;
        }
    }

    Ok(())
}

async fn cmd_start(proxy_port: u16, control_port: u16, serve_only: bool) -> Result<()> {
    let proxy = ProxyServer::new(proxy_port);

    // Configure proxy with default provider from DB
    if let Ok(repo) = legion_db::open_db() {
        if let Ok(Some(provider)) = repo.get_default_provider() {
            let config = legion_core::ProxyConfig {
                target_url: Some(provider.base_url.clone()),
                api_key: provider.api_key.clone(),
                api_format: Some(provider.api_format.clone()),
                model: provider.models.as_ref().and_then(|m| m.first().cloned()),
            };
            proxy.update_config(config).await;
        }
    }

    // Start proxy server
    let (proxy_tx, proxy_rx) = tokio::sync::oneshot::channel();
    let proxy_config_ref = proxy.config_ref();
    tokio::spawn(async move {
        if let Err(e) = proxy.start_with_signal(Some(proxy_tx)).await {
            tracing::error!("Proxy error: {}", e);
        }
    });

    // Start control API
    let (control_tx, control_rx) = tokio::sync::oneshot::channel();
    let control_api = ProxyControlApi::new(proxy_config_ref, control_port);
    tokio::spawn(async move {
        if let Err(e) = control_api.start_with_signal(Some(control_tx)).await {
            tracing::error!("Control API error: {}", e);
        }
    });

    // Wait for servers
    let timeout = std::time::Duration::from_secs(5);
    tokio::time::timeout(timeout, async {
        proxy_rx.await.ok();
        control_rx.await.ok();
    })
    .await
    .ok();

    if serve_only {
        println!("Servers running - proxy :{}, control :{}", proxy_port, control_port);
        tokio::signal::ctrl_c().await?;
        return Ok(());
    }

    // Run the full TUI
    legion_tui::run(proxy_port, control_port).await?;

    Ok(())
}
```

**Step 2: Verify it compiles**

Run: `cargo build`

**Step 3: Commit**

```bash
git add crates/legion-cli/src/main.rs
git commit -m "refactor(cli): simplify to start + switch commands with embedded TUI"
```

---

### Task 8: Integration test

Run the full app and verify Claude Code renders correctly.

**Step 1: Build**

Run: `cargo build`
Expected: clean compile

**Step 2: Test serve-only mode**

Run: `cargo run -- start --serve-only`
Expected: "Servers running - proxy :18080, control :19080"

Verify with: `curl http://127.0.0.1:19080/legion/status`
Expected: JSON with configured status

**Step 3: Test full TUI**

Run: `cargo run -- start`
Expected:
- Header line: "Legion v0.1.0  [Provider → Model] ●"
- Bordered main area with Claude Code rendering (no ANSI garbling)
- Footer: "Ctrl+P: Menu │ Ctrl+Q: Quit"
- Ctrl+P opens centered popup overlay
- Keys forward to Claude Code in normal mode

**Step 4: Commit**

```bash
git add -A
git commit -m "feat: complete tui-term embedded terminal integration"
```

---

### Task 9 (Future): Squad mode

Not in scope for initial implementation. After single mode works:
- Add `Squad` subcommand back
- Modify `ui.rs` to split main area: leader (65%) + stacked workers (35%)
- Each sub-area gets its own `PseudoTerminal` + `vt100::Parser` + PTY
- Tab key cycles focus between panes
- Each PTY gets independent proxy port
