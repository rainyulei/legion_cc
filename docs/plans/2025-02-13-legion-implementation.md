# Legion (军团) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Build a Claude Code wrapper CLI with model switching, session management, and squad collaboration features.

**Architecture:** Rust workspace with 5 crates (cli, tui, daemon, core, db). Single window mode first, then extend to squad mode. Unix Socket IPC + SQLite for persistence. HTTP proxy intercepts Claude Code requests and forwards to configured providers.

**Tech Stack:** Rust, ratatui (TUI), hyper (HTTP proxy), tokio (async runtime), rusqlite (SQLite), portable-pty (PTY embedding)

---

## Phase 1: Project Skeleton & Basic TUI

### Task 1: Initialize Workspace

**Files:**
- Create: `Cargo.toml`
- Create: `crates/legion-cli/Cargo.toml`
- Create: `crates/legion-cli/src/main.rs`
- Create: `crates/legion-tui/Cargo.toml`
- Create: `crates/legion-tui/src/lib.rs`
- Create: `crates/legion-core/Cargo.toml`
- Create: `crates/legion-core/src/lib.rs`
- Create: `crates/legion-db/Cargo.toml`
- Create: `crates/legion-db/src/lib.rs`
- Create: `crates/legion-daemon/Cargo.toml`
- Create: `crates/legion-daemon/src/lib.rs`

**Step 1: Create workspace Cargo.toml**

```toml
[workspace]
resolver = "2"
members = [
    "crates/legion-cli",
    "crates/legion-tui",
    "crates/legion-core",
    "crates/legion-db",
    "crates/legion-daemon",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"
authors = ["rainlei"]

[workspace.dependencies]
# Async runtime
tokio = { version = "1", features = ["full"] }

# TUI
ratatui = "0.29"
crossterm = "0.28"

# HTTP
hyper = { version = "1", features = ["full"] }
hyper-util = { version = "0.1", features = ["full"] }
http-body-util = "0.1"
reqwest = { version = "0.12", features = ["json"] }

# Database
rusqlite = { version = "0.32", features = ["bundled"] }

# PTY
portable-pty = "0.8"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# CLI
clap = { version = "4", features = ["derive"] }

# Utils
anyhow = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
uuid = { version = "1", features = ["v4"] }
chrono = { version = "0.4", features = ["serde"] }
dirs = "6"
```

**Step 2: Create legion-cli crate**

`crates/legion-cli/Cargo.toml`:
```toml
[package]
name = "legion-cli"
version.workspace = true
edition.workspace = true

[[bin]]
name = "legion"
path = "src/main.rs"

[dependencies]
legion-tui = { path = "../legion-tui" }
legion-core = { path = "../legion-core" }
legion-db = { path = "../legion-db" }
legion-daemon = { path = "../legion-daemon" }

clap.workspace = true
tokio.workspace = true
anyhow.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
```

`crates/legion-cli/src/main.rs`:
```rust
use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "legion")]
#[command(about = "Claude Code wrapper with model switching and squad mode")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start legion in single window mode
    Start,
    /// Start legion in squad mode
    Squad {
        #[arg(short, long, default_value = "3")]
        workers: u32,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Start) | None => {
            println!("Starting Legion in single window mode...");
            // TODO: Start TUI
        }
        Some(Commands::Squad { workers }) => {
            println!("Starting Legion squad mode with {} workers...", workers);
            // TODO: Start squad mode
        }
    }

    Ok(())
}
```

**Step 3: Create other crate stubs**

`crates/legion-tui/Cargo.toml`:
```toml
[package]
name = "legion-tui"
version.workspace = true
edition.workspace = true

[dependencies]
legion-core = { path = "../legion-core" }
legion-db = { path = "../legion-db" }

ratatui.workspace = true
crossterm.workspace = true
tokio.workspace = true
anyhow.workspace = true
```

`crates/legion-tui/src/lib.rs`:
```rust
pub mod app;
pub mod ui;
pub mod input;
```

`crates/legion-core/Cargo.toml`:
```toml
[package]
name = "legion-core"
version.workspace = true
edition.workspace = true

[dependencies]
tokio.workspace = true
hyper.workspace = true
hyper-util.workspace = true
http-body-util.workspace = true
reqwest.workspace = true
serde.workspace = true
serde_json.workspace = true
anyhow.workspace = true
thiserror.workspace = true
tracing.workspace = true
```

`crates/legion-core/src/lib.rs`:
```rust
pub mod proxy;
pub mod session;
pub mod ipc;
pub mod squad;
```

`crates/legion-db/Cargo.toml`:
```toml
[package]
name = "legion-db"
version.workspace = true
edition.workspace = true

[dependencies]
rusqlite.workspace = true
serde.workspace = true
serde_json.workspace = true
anyhow.workspace = true
chrono.workspace = true
dirs.workspace = true
```

`crates/legion-db/src/lib.rs`:
```rust
pub mod schema;
pub mod repo;
```

`crates/legion-daemon/Cargo.toml`:
```toml
[package]
name = "legion-daemon"
version.workspace = true
edition.workspace = true

[dependencies]
legion-core = { path = "../legion-core" }
legion-db = { path = "../legion-db" }

tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
anyhow.workspace = true
tracing.workspace = true
```

`crates/legion-daemon/src/lib.rs`:
```rust
pub mod server;
pub mod router;
```

**Step 4: Verify project builds**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo build`
Expected: Build succeeds with warnings about unused modules

**Step 5: Commit**

```bash
git init
git add .
git commit -m "feat: initialize legion workspace with 5 crates"
```

---

### Task 2: Database Schema & Repository

**Files:**
- Create: `crates/legion-db/src/schema.rs`
- Create: `crates/legion-db/src/repo.rs`
- Modify: `crates/legion-db/src/lib.rs`

**Step 1: Write database schema**

`crates/legion-db/src/schema.rs`:
```rust
use rusqlite::Connection;
use anyhow::Result;

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS providers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    api_key TEXT,
    api_format TEXT DEFAULT 'anthropic',
    models TEXT,
    is_default INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    name TEXT,
    project_path TEXT,
    claude_session_file TEXT,
    provider_id TEXT,
    created_at INTEGER NOT NULL,
    last_active_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS pending_questions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    worker_id TEXT NOT NULL,
    risk_level TEXT NOT NULL,
    content TEXT NOT NULL,
    context TEXT,
    status TEXT DEFAULT 'pending',
    answer TEXT,
    created_at INTEGER NOT NULL,
    answered_at INTEGER
);

CREATE TABLE IF NOT EXISTS workers (
    id TEXT PRIMARY KEY,
    role TEXT NOT NULL,
    status TEXT DEFAULT 'idle',
    current_task TEXT,
    provider_id TEXT,
    session_id TEXT,
    proxy_port INTEGER,
    pid INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
"#;

pub fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
    Ok(())
}
```

**Step 2: Write repository layer**

`crates/legion-db/src/repo.rs`:
```rust
use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub api_format: String,
    pub models: Option<Vec<String>>,
    pub is_default: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: Option<String>,
    pub project_path: Option<String>,
    pub claude_session_file: Option<String>,
    pub provider_id: Option<String>,
    pub created_at: i64,
    pub last_active_at: i64,
}

pub struct Repository {
    conn: Connection,
}

impl Repository {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }

    // Provider methods
    pub fn list_providers(&self) -> Result<Vec<Provider>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, base_url, api_key, api_format, models, is_default, created_at FROM providers ORDER BY name"
        )?;
        let rows = stmt.query_map([], |row| {
            let models_json: Option<String> = row.get(5)?;
            let models = models_json.and_then(|s| serde_json::from_str(&s).ok());
            Ok(Provider {
                id: row.get(0)?,
                name: row.get(1)?,
                base_url: row.get(2)?,
                api_key: row.get(3)?,
                api_format: row.get(4)?,
                models,
                is_default: row.get::<_, i32>(6)? != 0,
                created_at: row.get(7)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_provider(&self, id: &str) -> Result<Option<Provider>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, base_url, api_key, api_format, models, is_default, created_at FROM providers WHERE id = ?"
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            let models_json: Option<String> = row.get(5)?;
            let models = models_json.and_then(|s| serde_json::from_str(&s).ok());
            Ok(Some(Provider {
                id: row.get(0)?,
                name: row.get(1)?,
                base_url: row.get(2)?,
                api_key: row.get(3)?,
                api_format: row.get(4)?,
                models,
                is_default: row.get::<_, i32>(6)? != 0,
                created_at: row.get(7)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn insert_provider(&self, provider: &Provider) -> Result<()> {
        let models_json = provider.models.as_ref().map(|m| serde_json::to_string(m).unwrap());
        self.conn.execute(
            "INSERT INTO providers (id, name, base_url, api_key, api_format, models, is_default, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                provider.id,
                provider.name,
                provider.base_url,
                provider.api_key,
                provider.api_format,
                models_json,
                provider.is_default as i32,
                provider.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_provider_models(&self, id: &str, models: &[String]) -> Result<()> {
        let models_json = serde_json::to_string(models)?;
        self.conn.execute(
            "UPDATE providers SET models = ?1 WHERE id = ?2",
            params![models_json, id],
        )?;
        Ok(())
    }

    pub fn get_default_provider(&self) -> Result<Option<Provider>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, base_url, api_key, api_format, models, is_default, created_at FROM providers WHERE is_default = 1 LIMIT 1"
        )?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let models_json: Option<String> = row.get(5)?;
            let models = models_json.and_then(|s| serde_json::from_str(&s).ok());
            Ok(Some(Provider {
                id: row.get(0)?,
                name: row.get(1)?,
                base_url: row.get(2)?,
                api_key: row.get(3)?,
                api_format: row.get(4)?,
                models,
                is_default: true,
                created_at: row.get(7)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn set_default_provider(&self, id: &str) -> Result<()> {
        self.conn.execute("UPDATE providers SET is_default = 0", [])?;
        self.conn.execute("UPDATE providers SET is_default = 1 WHERE id = ?", params![id])?;
        Ok(())
    }

    // Session methods
    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, project_path, claude_session_file, provider_id, created_at, last_active_at FROM sessions ORDER BY last_active_at DESC"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Session {
                id: row.get(0)?,
                name: row.get(1)?,
                project_path: row.get(2)?,
                claude_session_file: row.get(3)?,
                provider_id: row.get(4)?,
                created_at: row.get(5)?,
                last_active_at: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn insert_session(&self, session: &Session) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, name, project_path, claude_session_file, provider_id, created_at, last_active_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session.id,
                session.name,
                session.project_path,
                session.claude_session_file,
                session.provider_id,
                session.created_at,
                session.last_active_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_session_active(&self, id: &str, timestamp: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET last_active_at = ?1 WHERE id = ?2",
            params![timestamp, id],
        )?;
        Ok(())
    }
}
```

**Step 3: Update lib.rs with exports**

`crates/legion-db/src/lib.rs`:
```rust
pub mod schema;
pub mod repo;

use anyhow::Result;
use rusqlite::Connection;
use std::path::PathBuf;

pub use repo::{Provider, Repository, Session};

pub fn get_db_path() -> PathBuf {
    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("legion");
    std::fs::create_dir_all(&data_dir).ok();
    data_dir.join("legion.db")
}

pub fn open_db() -> Result<Repository> {
    let path = get_db_path();
    let conn = Connection::open(&path)?;
    schema::init_db(&conn)?;
    Ok(Repository::new(conn))
}
```

**Step 4: Verify build**

Run: `cargo build -p legion-db`
Expected: Build succeeds

**Step 5: Commit**

```bash
git add .
git commit -m "feat(db): add schema and repository for providers/sessions"
```

---

### Task 3: Basic TUI Framework

**Files:**
- Create: `crates/legion-tui/src/app.rs`
- Create: `crates/legion-tui/src/ui.rs`
- Create: `crates/legion-tui/src/input.rs`
- Modify: `crates/legion-tui/src/lib.rs`

**Step 1: Create app state**

`crates/legion-tui/src/app.rs`:
```rust
use legion_db::{Provider, Session};

#[derive(Debug, Clone, PartialEq)]
pub enum PopupMenu {
    Main,
    Provider,
    Model,
    Session,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Normal,
    Popup(PopupMenu),
}

pub struct App {
    pub mode: AppMode,
    pub should_quit: bool,

    // Provider state
    pub providers: Vec<Provider>,
    pub current_provider: Option<Provider>,
    pub current_model: Option<String>,
    pub provider_connected: bool,

    // Session state
    pub sessions: Vec<Session>,
    pub current_session: Option<Session>,

    // Menu state
    pub menu_index: usize,
    pub submenu_index: usize,
}

impl App {
    pub fn new() -> Self {
        Self {
            mode: AppMode::Normal,
            should_quit: false,
            providers: Vec::new(),
            current_provider: None,
            current_model: None,
            provider_connected: false,
            sessions: Vec::new(),
            current_session: None,
            menu_index: 0,
            submenu_index: 0,
        }
    }

    pub fn main_menu_items(&self) -> Vec<(&str, String)> {
        vec![
            ("Provider", self.current_provider.as_ref()
                .map(|p| format!("{} {}", p.name, if self.provider_connected { "●" } else { "○" }))
                .unwrap_or_else(|| "未连接".to_string())),
            ("Model", self.current_model.clone().unwrap_or_else(|| "-".to_string())),
            ("Session", self.current_session.as_ref()
                .and_then(|s| s.name.clone())
                .unwrap_or_else(|| "无".to_string())),
            ("Settings", String::new()),
            ("Quit", String::new()),
        ]
    }

    pub fn toggle_popup(&mut self) {
        self.mode = match &self.mode {
            AppMode::Normal => {
                self.menu_index = 0;
                AppMode::Popup(PopupMenu::Main)
            }
            AppMode::Popup(_) => AppMode::Normal,
        };
    }

    pub fn enter_submenu(&mut self) {
        if let AppMode::Popup(PopupMenu::Main) = &self.mode {
            self.submenu_index = 0;
            self.mode = match self.menu_index {
                0 => AppMode::Popup(PopupMenu::Provider),
                1 => AppMode::Popup(PopupMenu::Model),
                2 => AppMode::Popup(PopupMenu::Session),
                4 => {
                    self.should_quit = true;
                    return;
                }
                _ => return,
            };
        }
    }

    pub fn back_to_main_menu(&mut self) {
        if let AppMode::Popup(_) = &self.mode {
            self.mode = AppMode::Popup(PopupMenu::Main);
            self.submenu_index = 0;
        }
    }

    pub fn menu_up(&mut self) {
        match &self.mode {
            AppMode::Popup(PopupMenu::Main) => {
                let len = self.main_menu_items().len();
                self.menu_index = (self.menu_index + len - 1) % len;
            }
            AppMode::Popup(PopupMenu::Provider) => {
                let len = self.providers.len().max(1);
                self.submenu_index = (self.submenu_index + len - 1) % len;
            }
            AppMode::Popup(PopupMenu::Model) => {
                if let Some(provider) = &self.current_provider {
                    if let Some(models) = &provider.models {
                        let len = models.len().max(1);
                        self.submenu_index = (self.submenu_index + len - 1) % len;
                    }
                }
            }
            AppMode::Popup(PopupMenu::Session) => {
                let len = (self.sessions.len() + 1).max(1); // +1 for "new session"
                self.submenu_index = (self.submenu_index + len - 1) % len;
            }
            _ => {}
        }
    }

    pub fn menu_down(&mut self) {
        match &self.mode {
            AppMode::Popup(PopupMenu::Main) => {
                let len = self.main_menu_items().len();
                self.menu_index = (self.menu_index + 1) % len;
            }
            AppMode::Popup(PopupMenu::Provider) => {
                let len = self.providers.len().max(1);
                self.submenu_index = (self.submenu_index + 1) % len;
            }
            AppMode::Popup(PopupMenu::Model) => {
                if let Some(provider) = &self.current_provider {
                    if let Some(models) = &provider.models {
                        let len = models.len().max(1);
                        self.submenu_index = (self.submenu_index + 1) % len;
                    }
                }
            }
            AppMode::Popup(PopupMenu::Session) => {
                let len = (self.sessions.len() + 1).max(1);
                self.submenu_index = (self.submenu_index + 1) % len;
            }
            _ => {}
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
```

**Step 2: Create UI rendering**

`crates/legion-tui/src/ui.rs`:
```rust
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::app::{App, AppMode, PopupMenu};

pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),  // Header
            Constraint::Min(0),     // Main content (PTY)
            Constraint::Length(1),  // Footer
        ])
        .split(frame.area());

    draw_header(frame, app, chunks[0]);
    draw_main(frame, app, chunks[1]);
    draw_footer(frame, chunks[2]);

    // Draw popup if active
    if let AppMode::Popup(menu) = &app.mode {
        draw_popup(frame, app, menu);
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let provider_info = app.current_provider.as_ref()
        .map(|p| {
            let model = app.current_model.as_deref().unwrap_or("-");
            let status = if app.provider_connected { "●" } else { "○" };
            format!("[{} → {}] {}", p.name, model, status)
        })
        .unwrap_or_else(|| "[未连接]".to_string());

    let header = Paragraph::new(Line::from(vec![
        Span::styled("Legion v0.1.0", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("        "),
        Span::styled(provider_info, Style::default().fg(Color::Green)),
    ]));
    frame.render_widget(header, area);
}

fn draw_main(frame: &mut Frame, _app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Claude Code ");
    let inner = Paragraph::new("PTY output will be rendered here...")
        .block(block);
    frame.render_widget(inner, area);
}

fn draw_footer(frame: &mut Frame, area: Rect) {
    let footer = Paragraph::new(Line::from(vec![
        Span::styled("Ctrl+P", Style::default().fg(Color::Yellow)),
        Span::raw(": 菜单 │ "),
        Span::styled("Ctrl+Q", Style::default().fg(Color::Yellow)),
        Span::raw(": 退出"),
    ]));
    frame.render_widget(footer, area);
}

fn draw_popup(frame: &mut Frame, app: &App, menu: &PopupMenu) {
    let area = centered_rect(50, 60, frame.area());
    frame.render_widget(Clear, area);

    match menu {
        PopupMenu::Main => draw_main_menu(frame, app, area),
        PopupMenu::Provider => draw_provider_menu(frame, app, area),
        PopupMenu::Model => draw_model_menu(frame, app, area),
        PopupMenu::Session => draw_session_menu(frame, app, area),
    }
}

fn draw_main_menu(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app.main_menu_items()
        .iter()
        .enumerate()
        .map(|(i, (label, value))| {
            let style = if i == app.menu_index {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let prefix = if i == app.menu_index { "> " } else { "  " };
            let text = if value.is_empty() {
                format!("{}{}", prefix, label)
            } else {
                format!("{}{}     [{}]", prefix, label, value)
            };
            ListItem::new(text).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default()
            .borders(Borders::ALL)
            .title(" Legion [ESC] "));
    frame.render_widget(list, area);
}

fn draw_provider_menu(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app.providers
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let style = if i == app.submenu_index {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let prefix = if i == app.submenu_index { "> " } else { "  " };
            let connected = app.current_provider.as_ref()
                .map(|cp| cp.id == p.id && app.provider_connected)
                .unwrap_or(false);
            let status = if connected { "[●]" } else { "[○]" };
            ListItem::new(format!("{}{} {}", prefix, status, p.name)).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default()
            .borders(Borders::ALL)
            .title(" Provider [ESC 返回] "));
    frame.render_widget(list, area);
}

fn draw_model_menu(frame: &mut Frame, app: &App, area: Rect) {
    let models = app.current_provider.as_ref()
        .and_then(|p| p.models.as_ref())
        .cloned()
        .unwrap_or_default();

    let items: Vec<ListItem> = models
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let style = if i == app.submenu_index {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let prefix = if i == app.submenu_index { "> " } else { "  " };
            let selected = app.current_model.as_ref().map(|cm| cm == m).unwrap_or(false);
            let check = if selected { "[*]" } else { "[ ]" };
            ListItem::new(format!("{}{} {}", prefix, check, m)).style(style)
        })
        .collect();

    let title = app.current_provider.as_ref()
        .map(|p| format!(" Model [{}] [ESC 返回] ", p.name))
        .unwrap_or_else(|| " Model [ESC 返回] ".to_string());

    let list = List::new(items)
        .block(Block::default()
            .borders(Borders::ALL)
            .title(title));
    frame.render_widget(list, area);
}

fn draw_session_menu(frame: &mut Frame, app: &App, area: Rect) {
    let mut items: Vec<ListItem> = app.sessions
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let style = if i == app.submenu_index {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let prefix = if i == app.submenu_index { "> " } else { "  " };
            let selected = app.current_session.as_ref().map(|cs| cs.id == s.id).unwrap_or(false);
            let check = if selected { "[*]" } else { "[ ]" };
            let name = s.name.as_deref().unwrap_or("unnamed");
            ListItem::new(format!("{}{} {}", prefix, check, name)).style(style)
        })
        .collect();

    // Add "new session" option
    let new_idx = app.sessions.len();
    let new_style = if new_idx == app.submenu_index {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let new_prefix = if new_idx == app.submenu_index { "> " } else { "  " };
    items.push(ListItem::new(format!("{}[+] 新建 Session", new_prefix)).style(new_style));

    let list = List::new(items)
        .block(Block::default()
            .borders(Borders::ALL)
            .title(" Session [ESC 返回] "));
    frame.render_widget(list, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
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
```

**Step 3: Create input handler**

`crates/legion-tui/src/input.rs`:
```rust
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use crate::app::{App, AppMode, PopupMenu};

pub enum InputResult {
    Continue,
    Quit,
}

pub fn handle_key(app: &mut App, key: KeyEvent) -> InputResult {
    // Global shortcuts
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
    match &app.mode {
        AppMode::Normal => handle_normal_mode(app, key),
        AppMode::Popup(_) => handle_popup_mode(app, key),
    }
}

fn handle_normal_mode(_app: &mut App, _key: KeyEvent) -> InputResult {
    // In normal mode, keys go to PTY
    InputResult::Continue
}

fn handle_popup_mode(app: &mut App, key: KeyEvent) -> InputResult {
    match key.code {
        KeyCode::Esc => {
            match &app.mode {
                AppMode::Popup(PopupMenu::Main) => app.mode = AppMode::Normal,
                AppMode::Popup(_) => app.back_to_main_menu(),
                _ => {}
            }
        }
        KeyCode::Up | KeyCode::Char('k') => app.menu_up(),
        KeyCode::Down | KeyCode::Char('j') => app.menu_down(),
        KeyCode::Enter => {
            match &app.mode {
                AppMode::Popup(PopupMenu::Main) => app.enter_submenu(),
                AppMode::Popup(PopupMenu::Provider) => {
                    // Select provider
                    if app.submenu_index < app.providers.len() {
                        let provider = app.providers[app.submenu_index].clone();
                        app.current_provider = Some(provider);
                        app.provider_connected = false; // Need to connect
                        // TODO: Trigger connection
                    }
                    app.back_to_main_menu();
                }
                AppMode::Popup(PopupMenu::Model) => {
                    // Select model
                    if let Some(provider) = &app.current_provider {
                        if let Some(models) = &provider.models {
                            if app.submenu_index < models.len() {
                                app.current_model = Some(models[app.submenu_index].clone());
                            }
                        }
                    }
                    app.back_to_main_menu();
                }
                AppMode::Popup(PopupMenu::Session) => {
                    // Select or create session
                    if app.submenu_index < app.sessions.len() {
                        let session = app.sessions[app.submenu_index].clone();
                        app.current_session = Some(session);
                    } else {
                        // TODO: Create new session
                    }
                    app.back_to_main_menu();
                }
                _ => {}
            }
        }
        _ => {}
    }

    if app.should_quit {
        InputResult::Quit
    } else {
        InputResult::Continue
    }
}
```

**Step 4: Update lib.rs**

`crates/legion-tui/src/lib.rs`:
```rust
pub mod app;
pub mod ui;
pub mod input;

use std::io;
use anyhow::Result;
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;

use app::App;
use input::{handle_key, InputResult};

pub async fn run() -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app
    let mut app = App::new();

    // Load initial data
    if let Ok(repo) = legion_db::open_db() {
        app.providers = repo.list_providers().unwrap_or_default();
        app.sessions = repo.list_sessions().unwrap_or_default();
        if let Ok(Some(provider)) = repo.get_default_provider() {
            app.current_provider = Some(provider);
        }
    }

    // Main loop
    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match handle_key(&mut app, key) {
                    InputResult::Quit => break,
                    InputResult::Continue => {}
                }
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    Ok(())
}
```

**Step 5: Update CLI to use TUI**

`crates/legion-cli/src/main.rs`:
```rust
use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "legion")]
#[command(about = "Claude Code wrapper with model switching and squad mode")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start legion in single window mode
    Start,
    /// Start legion in squad mode
    Squad {
        #[arg(short, long, default_value = "3")]
        workers: u32,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Start) | None => {
            legion_tui::run().await?;
        }
        Some(Commands::Squad { workers }) => {
            println!("Starting Legion squad mode with {} workers...", workers);
            // TODO: Start squad mode
        }
    }

    Ok(())
}
```

**Step 6: Verify build and test**

Run: `cargo build && cargo run -p legion-cli`
Expected: TUI starts, Ctrl+P opens menu, ESC closes, Ctrl+Q quits

**Step 7: Commit**

```bash
git add .
git commit -m "feat(tui): add basic TUI with popup menu system"
```

---

## Phase 2: HTTP Proxy & Provider Connection

### Task 4: HTTP Proxy Core

**Files:**
- Create: `crates/legion-core/src/proxy/mod.rs`
- Create: `crates/legion-core/src/proxy/server.rs`
- Create: `crates/legion-core/src/proxy/transform.rs`
- Modify: `crates/legion-core/src/lib.rs`

**Step 1: Create proxy module structure**

`crates/legion-core/src/proxy/mod.rs`:
```rust
pub mod server;
pub mod transform;

pub use server::ProxyServer;
```

**Step 2: Create proxy server**

`crates/legion-core/src/proxy/server.rs`:
```rust
use std::net::SocketAddr;
use std::sync::Arc;
use anyhow::Result;
use hyper::{body::Incoming, server::conn::http1, service::service_fn, Request, Response};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use http_body_util::{BodyExt, Full};
use bytes::Bytes;

use super::transform;

#[derive(Clone)]
pub struct ProxyConfig {
    pub target_url: String,
    pub api_key: Option<String>,
    pub api_format: String, // "anthropic" or "openai_chat"
    pub model: Option<String>,
}

pub struct ProxyServer {
    config: Arc<RwLock<ProxyConfig>>,
    port: u16,
}

impl ProxyServer {
    pub fn new(port: u16) -> Self {
        Self {
            config: Arc::new(RwLock::new(ProxyConfig {
                target_url: String::new(),
                api_key: None,
                api_format: "anthropic".to_string(),
                model: None,
            })),
            port,
        }
    }

    pub async fn update_config(&self, config: ProxyConfig) {
        let mut guard = self.config.write().await;
        *guard = config;
    }

    pub async fn get_config(&self) -> ProxyConfig {
        self.config.read().await.clone()
    }

    pub async fn start(&self) -> Result<()> {
        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        let listener = TcpListener::bind(addr).await?;
        tracing::info!("Proxy listening on {}", addr);

        let config = self.config.clone();

        loop {
            let (stream, _) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let config = config.clone();

            tokio::spawn(async move {
                let service = service_fn(move |req| {
                    let config = config.clone();
                    async move { handle_request(req, config).await }
                });

                if let Err(e) = http1::Builder::new().serve_connection(io, service).await {
                    tracing::error!("Connection error: {}", e);
                }
            });
        }
    }
}

async fn handle_request(
    req: Request<Incoming>,
    config: Arc<RwLock<ProxyConfig>>,
) -> Result<Response<Full<Bytes>>, hyper::Error> {
    let config = config.read().await.clone();

    if config.target_url.is_empty() {
        return Ok(Response::builder()
            .status(503)
            .body(Full::new(Bytes::from("No provider configured")))
            .unwrap());
    }

    // Collect request body
    let (parts, body) = req.into_parts();
    let body_bytes = body.collect().await?.to_bytes();

    // Transform if needed
    let (target_url, transformed_body, extra_headers) = if config.api_format == "openai_chat" {
        let endpoint = if parts.uri.path() == "/v1/messages" {
            "/chat/completions"
        } else {
            parts.uri.path()
        };
        let target = format!("{}{}", config.target_url.trim_end_matches('/'), endpoint);
        let transformed = transform::anthropic_to_openai(&body_bytes, config.model.as_deref())
            .unwrap_or_else(|_| body_bytes.to_vec());
        (target, transformed, vec![("Content-Type", "application/json")])
    } else {
        let target = format!("{}{}", config.target_url.trim_end_matches('/'), parts.uri.path());
        (target, body_bytes.to_vec(), vec![])
    };

    // Build outgoing request
    let client = reqwest::Client::new();
    let mut builder = client.request(
        parts.method.as_str().parse().unwrap(),
        &target_url,
    );

    // Copy headers
    for (name, value) in parts.headers.iter() {
        if name != "host" && name != "content-length" {
            if let Ok(v) = value.to_str() {
                builder = builder.header(name.as_str(), v);
            }
        }
    }

    // Add API key
    if let Some(key) = &config.api_key {
        builder = builder.header("Authorization", format!("Bearer {}", key));
    }

    // Add extra headers
    for (name, value) in extra_headers {
        builder = builder.header(name, value);
    }

    // Send request
    let response = match builder.body(transformed_body).send().await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::error!("Proxy request failed: {}", e);
            return Ok(Response::builder()
                .status(502)
                .body(Full::new(Bytes::from(format!("Proxy error: {}", e))))
                .unwrap());
        }
    };

    // Get response
    let status = response.status();
    let response_body = response.bytes().await.unwrap_or_default();

    // Transform response if needed
    let final_body = if config.api_format == "openai_chat" {
        transform::openai_to_anthropic(&response_body)
            .unwrap_or_else(|_| response_body.to_vec())
    } else {
        response_body.to_vec()
    };

    Ok(Response::builder()
        .status(status.as_u16())
        .body(Full::new(Bytes::from(final_body)))
        .unwrap())
}
```

**Step 3: Create transform module**

`crates/legion-core/src/proxy/transform.rs`:
```rust
use anyhow::Result;
use serde_json::{json, Value};

/// Transform Anthropic Messages API request to OpenAI Chat API format
pub fn anthropic_to_openai(body: &[u8], model_override: Option<&str>) -> Result<Vec<u8>> {
    let anthropic: Value = serde_json::from_slice(body)?;

    let model = model_override
        .map(String::from)
        .or_else(|| anthropic["model"].as_str().map(String::from))
        .unwrap_or_else(|| "gpt-4".to_string());

    let mut messages = Vec::new();

    // Convert system message
    if let Some(system) = anthropic["system"].as_str() {
        messages.push(json!({
            "role": "system",
            "content": system
        }));
    }

    // Convert messages
    if let Some(msgs) = anthropic["messages"].as_array() {
        for msg in msgs {
            let role = msg["role"].as_str().unwrap_or("user");
            let content = if let Some(content_arr) = msg["content"].as_array() {
                // Handle content blocks
                content_arr.iter()
                    .filter_map(|c| c["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            } else {
                msg["content"].as_str().unwrap_or("").to_string()
            };

            messages.push(json!({
                "role": role,
                "content": content
            }));
        }
    }

    let openai = json!({
        "model": model,
        "messages": messages,
        "max_tokens": anthropic["max_tokens"].as_u64().unwrap_or(4096),
        "stream": anthropic["stream"].as_bool().unwrap_or(false)
    });

    Ok(serde_json::to_vec(&openai)?)
}

/// Transform OpenAI Chat API response to Anthropic Messages API format
pub fn openai_to_anthropic(body: &[u8]) -> Result<Vec<u8>> {
    let openai: Value = serde_json::from_slice(body)?;

    // Handle error responses
    if openai.get("error").is_some() {
        return Ok(body.to_vec());
    }

    let choices = openai["choices"].as_array();
    let content = choices
        .and_then(|c| c.first())
        .and_then(|c| c["message"]["content"].as_str())
        .unwrap_or("");

    let anthropic = json!({
        "id": openai["id"].as_str().unwrap_or("msg_proxy"),
        "type": "message",
        "role": "assistant",
        "content": [{
            "type": "text",
            "text": content
        }],
        "model": openai["model"].as_str().unwrap_or("unknown"),
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": openai["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
            "output_tokens": openai["usage"]["completion_tokens"].as_u64().unwrap_or(0)
        }
    });

    Ok(serde_json::to_vec(&anthropic)?)
}
```

**Step 4: Update lib.rs**

`crates/legion-core/src/lib.rs`:
```rust
pub mod proxy;
pub mod session;
pub mod ipc;
pub mod squad;

pub use proxy::ProxyServer;
```

**Step 5: Add bytes dependency**

Add to `crates/legion-core/Cargo.toml`:
```toml
bytes = "1"
```

**Step 6: Verify build**

Run: `cargo build -p legion-core`
Expected: Build succeeds

**Step 7: Commit**

```bash
git add .
git commit -m "feat(core): add HTTP proxy with Anthropic/OpenAI transform"
```

---

### Task 5: Integrate Proxy with TUI

**Files:**
- Modify: `crates/legion-tui/Cargo.toml`
- Modify: `crates/legion-tui/src/lib.rs`
- Modify: `crates/legion-tui/src/app.rs`

**Step 1: Add legion-core dependency**

`crates/legion-tui/Cargo.toml`:
```toml
[dependencies]
legion-core = { path = "../legion-core" }
legion-db = { path = "../legion-db" }
# ... rest unchanged
```

**Step 2: Update app.rs with proxy state**

Add to `crates/legion-tui/src/app.rs`:
```rust
use std::sync::Arc;
use legion_core::proxy::{ProxyServer, ProxyConfig};

pub struct App {
    // ... existing fields ...

    // Proxy
    pub proxy: Arc<ProxyServer>,
    pub proxy_port: u16,
}

impl App {
    pub fn new(proxy_port: u16) -> Self {
        Self {
            mode: AppMode::Normal,
            should_quit: false,
            providers: Vec::new(),
            current_provider: None,
            current_model: None,
            provider_connected: false,
            sessions: Vec::new(),
            current_session: None,
            menu_index: 0,
            submenu_index: 0,
            proxy: Arc::new(ProxyServer::new(proxy_port)),
            proxy_port,
        }
    }

    pub async fn connect_provider(&mut self) -> anyhow::Result<()> {
        if let Some(provider) = &self.current_provider {
            let config = ProxyConfig {
                target_url: provider.base_url.clone(),
                api_key: provider.api_key.clone(),
                api_format: provider.api_format.clone(),
                model: self.current_model.clone(),
            };
            self.proxy.update_config(config).await;
            self.provider_connected = true;

            // TODO: Fetch models from provider
        }
        Ok(())
    }

    pub async fn update_model(&mut self, model: String) {
        self.current_model = Some(model.clone());
        if let Some(provider) = &self.current_provider {
            let config = ProxyConfig {
                target_url: provider.base_url.clone(),
                api_key: provider.api_key.clone(),
                api_format: provider.api_format.clone(),
                model: Some(model),
            };
            self.proxy.update_config(config).await;
        }
    }
}
```

**Step 3: Update lib.rs to start proxy**

`crates/legion-tui/src/lib.rs`:
```rust
pub mod app;
pub mod ui;
pub mod input;

use std::io;
use std::sync::Arc;
use anyhow::Result;
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::prelude::*;
use tokio::sync::mpsc;

use app::App;
use input::{handle_key, InputResult};

pub async fn run() -> Result<()> {
    run_with_port(18080).await
}

pub async fn run_with_port(proxy_port: u16) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app
    let mut app = App::new(proxy_port);

    // Load initial data
    if let Ok(repo) = legion_db::open_db() {
        app.providers = repo.list_providers().unwrap_or_default();
        app.sessions = repo.list_sessions().unwrap_or_default();
        if let Ok(Some(provider)) = repo.get_default_provider() {
            app.current_provider = Some(provider);
        }
    }

    // Start proxy in background
    let proxy = app.proxy.clone();
    tokio::spawn(async move {
        if let Err(e) = proxy.start().await {
            tracing::error!("Proxy error: {}", e);
        }
    });

    // Main loop
    loop {
        terminal.draw(|f| ui::draw(f, &app))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match handle_key(&mut app, key) {
                    InputResult::Quit => break,
                    InputResult::Continue => {}
                }
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;

    Ok(())
}
```

**Step 4: Verify build**

Run: `cargo build`
Expected: Build succeeds

**Step 5: Commit**

```bash
git add .
git commit -m "feat(tui): integrate proxy server with TUI app"
```

---

## Phase 3: PTY Embedding (Claude Code)

### Task 6: PTY Module

**Files:**
- Create: `crates/legion-tui/src/pty.rs`
- Modify: `crates/legion-tui/src/lib.rs`
- Modify: `crates/legion-tui/src/app.rs`
- Modify: `crates/legion-tui/src/ui.rs`

**Step 1: Create PTY module**

`crates/legion-tui/src/pty.rs`:
```rust
use anyhow::Result;
use portable_pty::{native_pty_system, CommandBuilder, PtyPair, PtySize};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

pub struct PtyHandle {
    pair: PtyPair,
    output_buffer: Arc<Mutex<Vec<u8>>>,
    _reader_handle: std::thread::JoinHandle<()>,
}

impl PtyHandle {
    pub fn spawn_claude(proxy_port: u16) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new("claude");
        cmd.env("ANTHROPIC_BASE_URL", format!("http://127.0.0.1:{}", proxy_port));

        let _child = pair.slave.spawn_command(cmd)?;

        let output_buffer = Arc::new(Mutex::new(Vec::new()));
        let buffer_clone = output_buffer.clone();

        let mut reader = pair.master.try_clone_reader()?;

        let reader_handle = std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let mut guard = buffer_clone.lock().unwrap();
                        guard.extend_from_slice(&buf[..n]);
                        // Keep buffer size reasonable
                        if guard.len() > 100_000 {
                            guard.drain(..50_000);
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            pair,
            output_buffer,
            _reader_handle: reader_handle,
        })
    }

    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.pair.master.write_all(data)?;
        Ok(())
    }

    pub fn read_output(&self) -> Vec<u8> {
        let guard = self.output_buffer.lock().unwrap();
        guard.clone()
    }

    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.pair.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }
}
```

**Step 2: Update app.rs with PTY**

Add to `crates/legion-tui/src/app.rs`:
```rust
use crate::pty::PtyHandle;

pub struct App {
    // ... existing fields ...

    // PTY
    pub pty: Option<PtyHandle>,
    pub pty_output: String,
}

impl App {
    pub fn new(proxy_port: u16) -> Self {
        Self {
            // ... existing fields ...
            pty: None,
            pty_output: String::new(),
        }
    }

    pub fn start_claude(&mut self) -> anyhow::Result<()> {
        let pty = PtyHandle::spawn_claude(self.proxy_port)?;
        self.pty = Some(pty);
        Ok(())
    }

    pub fn update_pty_output(&mut self) {
        if let Some(pty) = &self.pty {
            let output = pty.read_output();
            self.pty_output = String::from_utf8_lossy(&output).to_string();
        }
    }

    pub fn send_to_pty(&mut self, data: &[u8]) {
        if let Some(pty) = &mut self.pty {
            let _ = pty.write(data);
        }
    }
}
```

**Step 3: Update ui.rs to show PTY output**

Modify `draw_main` in `crates/legion-tui/src/ui.rs`:
```rust
fn draw_main(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Claude Code ");

    // Show last N lines of PTY output
    let lines: Vec<&str> = app.pty_output.lines().rev().take(100).collect();
    let text: Vec<Line> = lines.iter().rev().map(|l| Line::from(*l)).collect();

    let inner = Paragraph::new(text)
        .block(block)
        .wrap(ratatui::widgets::Wrap { trim: false });

    frame.render_widget(inner, area);
}
```

**Step 4: Update input.rs to forward keys to PTY**

Modify `handle_normal_mode` in `crates/legion-tui/src/input.rs`:
```rust
fn handle_normal_mode(app: &mut App, key: KeyEvent) -> InputResult {
    // Forward keys to PTY
    let bytes = key_to_bytes(key);
    if !bytes.is_empty() {
        app.send_to_pty(&bytes);
    }
    InputResult::Continue
}

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
```

**Step 5: Update lib.rs main loop**

`crates/legion-tui/src/lib.rs`:
```rust
pub async fn run_with_port(proxy_port: u16) -> Result<()> {
    // ... setup code ...

    // Start proxy in background
    let proxy = app.proxy.clone();
    tokio::spawn(async move {
        if let Err(e) = proxy.start().await {
            tracing::error!("Proxy error: {}", e);
        }
    });

    // Wait a bit for proxy to start
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Start Claude Code
    if let Err(e) = app.start_claude() {
        tracing::warn!("Failed to start Claude: {}", e);
    }

    // Main loop
    loop {
        // Update PTY output
        app.update_pty_output();

        terminal.draw(|f| ui::draw(f, &app))?;

        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match handle_key(&mut app, key) {
                    InputResult::Quit => break,
                    InputResult::Continue => {}
                }
            }
        }
    }

    // ... cleanup code ...
}
```

**Step 6: Verify build**

Run: `cargo build`
Expected: Build succeeds

**Step 7: Commit**

```bash
git add .
git commit -m "feat(tui): add PTY embedding for Claude Code"
```

---

## Phase 4: Session Management

### Task 7: Session Discovery & Management

**Files:**
- Create: `crates/legion-core/src/session/mod.rs`
- Create: `crates/legion-core/src/session/discovery.rs`
- Modify: `crates/legion-tui/src/app.rs`

**Step 1: Create session discovery**

`crates/legion-core/src/session/mod.rs`:
```rust
pub mod discovery;
pub use discovery::*;
```

`crates/legion-core/src/session/discovery.rs`:
```rust
use anyhow::Result;
use std::path::PathBuf;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct ClaudeSession {
    pub id: String,
    pub project_path: String,
    pub session_file: PathBuf,
    pub last_modified: DateTime<Utc>,
}

pub fn get_claude_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
}

pub fn discover_sessions() -> Result<Vec<ClaudeSession>> {
    let claude_dir = get_claude_dir();
    let projects_dir = claude_dir.join("projects");

    let mut sessions = Vec::new();

    if !projects_dir.exists() {
        return Ok(sessions);
    }

    // Walk through projects directory
    for entry in std::fs::read_dir(&projects_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            // Look for .jsonl files
            for file_entry in std::fs::read_dir(&path)? {
                let file_entry = file_entry?;
                let file_path = file_entry.path();

                if file_path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                    let metadata = std::fs::metadata(&file_path)?;
                    let modified: DateTime<Utc> = metadata.modified()?.into();

                    let id = file_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string();

                    let project_path = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string();

                    sessions.push(ClaudeSession {
                        id,
                        project_path,
                        session_file: file_path,
                        last_modified: modified,
                    });
                }
            }
        }
    }

    // Sort by last modified (most recent first)
    sessions.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));

    Ok(sessions)
}

pub fn get_session_display_name(session: &ClaudeSession) -> String {
    let elapsed = Utc::now() - session.last_modified;
    let time_str = if elapsed.num_hours() < 1 {
        format!("{}m ago", elapsed.num_minutes())
    } else if elapsed.num_days() < 1 {
        format!("{}h ago", elapsed.num_hours())
    } else {
        format!("{}d ago", elapsed.num_days())
    };

    format!("{} ({})", session.project_path, time_str)
}
```

**Step 2: Update app.rs to use session discovery**

Add to `crates/legion-tui/src/app.rs`:
```rust
use legion_core::session::{discover_sessions, ClaudeSession, get_session_display_name};

pub struct App {
    // Replace sessions field type
    pub claude_sessions: Vec<ClaudeSession>,
    pub current_claude_session: Option<ClaudeSession>,
    // ... rest unchanged
}

impl App {
    pub fn refresh_sessions(&mut self) {
        self.claude_sessions = discover_sessions().unwrap_or_default();
    }

    pub fn switch_session(&mut self, session: &ClaudeSession) -> anyhow::Result<()> {
        self.current_claude_session = Some(session.clone());

        // Restart Claude with --resume pointing to the session file
        // This will require modifying PTY spawn
        Ok(())
    }
}
```

**Step 3: Verify build**

Run: `cargo build`
Expected: Build succeeds

**Step 4: Commit**

```bash
git add .
git commit -m "feat(core): add Claude session discovery"
```

---

## Phase 5: Squad Mode (IPC & Daemon)

### Task 8: IPC Protocol

**Files:**
- Create: `crates/legion-core/src/ipc/mod.rs`
- Create: `crates/legion-core/src/ipc/protocol.rs`
- Create: `crates/legion-core/src/ipc/client.rs`

**Step 1: Create IPC protocol**

`crates/legion-core/src/ipc/mod.rs`:
```rust
pub mod protocol;
pub mod client;

pub use protocol::*;
pub use client::*;
```

`crates/legion-core/src/ipc/protocol.rs`:
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Risk {
    Low,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerStatus {
    Idle,
    Busy,
    Waiting,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingQuestion {
    pub id: i64,
    pub worker_id: String,
    pub risk: Risk,
    pub content: String,
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Message {
    // Worker → Daemon
    WorkerReady {
        worker_id: String,
        role: String,
    },
    Question {
        worker_id: String,
        risk: Risk,
        content: String,
        context: Option<String>,
    },
    StatusUpdate {
        worker_id: String,
        status: WorkerStatus,
        current_task: Option<String>,
    },

    // Daemon → Worker
    Answer {
        question_id: i64,
        answer: String,
    },
    TaskAssign {
        task: String,
    },

    // Daemon → Leader
    NewQuestion {
        question: PendingQuestion,
    },
    WorkerStatusChanged {
        worker_id: String,
        status: WorkerStatus,
    },
    AllWorkersReady {
        count: usize,
    },

    // Generic
    Ping,
    Pong,
    Error {
        message: String,
    },
}

pub fn serialize_message(msg: &Message) -> Vec<u8> {
    let json = serde_json::to_string(msg).unwrap();
    let len = json.len() as u32;
    let mut buf = len.to_be_bytes().to_vec();
    buf.extend(json.as_bytes());
    buf
}

pub fn deserialize_message(data: &[u8]) -> Option<Message> {
    if data.len() < 4 {
        return None;
    }
    let len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if data.len() < 4 + len {
        return None;
    }
    serde_json::from_slice(&data[4..4 + len]).ok()
}
```

`crates/legion-core/src/ipc/client.rs`:
```rust
use anyhow::Result;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use super::protocol::{deserialize_message, serialize_message, Message};

pub fn get_socket_path() -> PathBuf {
    dirs::runtime_dir()
        .or_else(|| dirs::data_local_dir())
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("legion.sock")
}

pub struct IpcClient {
    stream: UnixStream,
}

impl IpcClient {
    pub async fn connect() -> Result<Self> {
        let path = get_socket_path();
        let stream = UnixStream::connect(&path).await?;
        Ok(Self { stream })
    }

    pub async fn send(&mut self, msg: Message) -> Result<()> {
        let data = serialize_message(&msg);
        self.stream.write_all(&data).await?;
        Ok(())
    }

    pub async fn recv(&mut self) -> Result<Message> {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;

        let mut data = vec![0u8; len];
        self.stream.read_exact(&mut data).await?;

        let mut full = len_buf.to_vec();
        full.extend(data);

        deserialize_message(&full)
            .ok_or_else(|| anyhow::anyhow!("Failed to deserialize message"))
    }
}
```

**Step 2: Verify build**

Run: `cargo build -p legion-core`
Expected: Build succeeds

**Step 3: Commit**

```bash
git add .
git commit -m "feat(core): add IPC protocol for squad mode"
```

---

### Task 9: Daemon Server

**Files:**
- Create: `crates/legion-daemon/src/server.rs`
- Create: `crates/legion-daemon/src/router.rs`
- Modify: `crates/legion-daemon/src/lib.rs`

**Step 1: Create daemon server**

`crates/legion-daemon/src/server.rs`:
```rust
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, RwLock};

use legion_core::ipc::{
    deserialize_message, get_socket_path, serialize_message, Message, PendingQuestion, Risk,
    WorkerStatus,
};
use legion_db::Repository;

pub struct WorkerState {
    pub id: String,
    pub role: String,
    pub status: WorkerStatus,
    pub current_task: Option<String>,
}

pub struct DaemonState {
    pub workers: HashMap<String, WorkerState>,
    pub pending_questions: Vec<PendingQuestion>,
    pub leader_tx: Option<mpsc::Sender<Message>>,
    pub next_question_id: i64,
}

impl DaemonState {
    pub fn new() -> Self {
        Self {
            workers: HashMap::new(),
            pending_questions: Vec::new(),
            leader_tx: None,
            next_question_id: 1,
        }
    }
}

pub struct DaemonServer {
    state: Arc<RwLock<DaemonState>>,
}

impl DaemonServer {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(DaemonState::new())),
        }
    }

    pub async fn start(&self) -> Result<()> {
        let socket_path = get_socket_path();

        // Remove existing socket
        let _ = std::fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path)?;
        tracing::info!("Daemon listening on {:?}", socket_path);

        loop {
            let (stream, _) = listener.accept().await?;
            let state = self.state.clone();

            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, state).await {
                    tracing::error!("Connection error: {}", e);
                }
            });
        }
    }
}

async fn handle_connection(
    mut stream: UnixStream,
    state: Arc<RwLock<DaemonState>>,
) -> Result<()> {
    let mut worker_id: Option<String> = None;

    loop {
        // Read message length
        let mut len_buf = [0u8; 4];
        if stream.read_exact(&mut len_buf).await.is_err() {
            break;
        }
        let len = u32::from_be_bytes(len_buf) as usize;

        // Read message body
        let mut data = vec![0u8; len];
        stream.read_exact(&mut data).await?;

        let mut full = len_buf.to_vec();
        full.extend(data);

        let msg = match deserialize_message(&full) {
            Some(m) => m,
            None => continue,
        };

        let response = process_message(msg, &state, &mut worker_id).await;

        if let Some(resp) = response {
            let resp_data = serialize_message(&resp);
            stream.write_all(&resp_data).await?;
        }
    }

    // Cleanup on disconnect
    if let Some(id) = worker_id {
        let mut guard = state.write().await;
        guard.workers.remove(&id);
    }

    Ok(())
}

async fn process_message(
    msg: Message,
    state: &Arc<RwLock<DaemonState>>,
    worker_id: &mut Option<String>,
) -> Option<Message> {
    match msg {
        Message::WorkerReady { worker_id: id, role } => {
            let mut guard = state.write().await;
            guard.workers.insert(
                id.clone(),
                WorkerState {
                    id: id.clone(),
                    role: role.clone(),
                    status: WorkerStatus::Idle,
                    current_task: None,
                },
            );
            *worker_id = Some(id);

            // Notify leader if all workers ready
            let worker_count = guard.workers.len();
            if let Some(tx) = &guard.leader_tx {
                let _ = tx
                    .send(Message::AllWorkersReady {
                        count: worker_count,
                    })
                    .await;
            }

            Some(Message::Pong)
        }

        Message::Question {
            worker_id: wid,
            risk,
            content,
            context,
        } => {
            let mut guard = state.write().await;
            let question_id = guard.next_question_id;
            guard.next_question_id += 1;

            let question = PendingQuestion {
                id: question_id,
                worker_id: wid,
                risk: risk.clone(),
                content,
                context,
            };

            match risk {
                Risk::Low => {
                    // Auto-answer low risk questions
                    // TODO: Use AI to decide
                    Some(Message::Answer {
                        question_id,
                        answer: "y".to_string(),
                    })
                }
                Risk::High => {
                    // Forward to leader
                    guard.pending_questions.push(question.clone());
                    if let Some(tx) = &guard.leader_tx {
                        let _ = tx.send(Message::NewQuestion { question }).await;
                    }
                    None // No immediate response, wait for leader
                }
            }
        }

        Message::StatusUpdate {
            worker_id: wid,
            status,
            current_task,
        } => {
            let mut guard = state.write().await;
            if let Some(worker) = guard.workers.get_mut(&wid) {
                worker.status = status.clone();
                worker.current_task = current_task;
            }

            // Notify leader
            if let Some(tx) = &guard.leader_tx {
                let _ = tx
                    .send(Message::WorkerStatusChanged {
                        worker_id: wid,
                        status,
                    })
                    .await;
            }

            None
        }

        Message::Ping => Some(Message::Pong),

        _ => None,
    }
}
```

**Step 2: Create router stub**

`crates/legion-daemon/src/router.rs`:
```rust
// Router for message distribution - to be expanded
pub struct MessageRouter;

impl MessageRouter {
    pub fn new() -> Self {
        Self
    }
}
```

**Step 3: Update lib.rs**

`crates/legion-daemon/src/lib.rs`:
```rust
pub mod server;
pub mod router;

pub use server::DaemonServer;
```

**Step 4: Verify build**

Run: `cargo build -p legion-daemon`
Expected: Build succeeds

**Step 5: Commit**

```bash
git add .
git commit -m "feat(daemon): add Unix socket daemon server"
```

---

### Task 10: Squad Mode Orchestration

**Files:**
- Create: `crates/legion-core/src/squad/mod.rs`
- Create: `crates/legion-core/src/squad/tmux.rs`
- Create: `crates/legion-core/src/squad/orchestrator.rs`
- Modify: `crates/legion-cli/src/main.rs`

**Step 1: Create tmux module**

`crates/legion-core/src/squad/mod.rs`:
```rust
pub mod tmux;
pub mod orchestrator;

pub use orchestrator::*;
```

`crates/legion-core/src/squad/tmux.rs`:
```rust
use anyhow::Result;
use std::process::Command;

pub struct TmuxLayout {
    pub session_name: String,
    pub leader_pane: String,
    pub worker_panes: Vec<String>,
}

pub fn create_squad_layout(session_name: &str, worker_count: u32) -> Result<TmuxLayout> {
    // Create new tmux session
    Command::new("tmux")
        .args(["new-session", "-d", "-s", session_name])
        .output()?;

    // Split vertically for leader (left) and workers (right)
    Command::new("tmux")
        .args(["split-window", "-h", "-t", session_name])
        .output()?;

    // Split workers horizontally
    for i in 1..worker_count {
        Command::new("tmux")
            .args([
                "split-window",
                "-v",
                "-t",
                &format!("{}:0.1", session_name),
            ])
            .output()?;
    }

    // Collect pane IDs
    let output = Command::new("tmux")
        .args(["list-panes", "-t", session_name, "-F", "#{pane_id}"])
        .output()?;

    let panes: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(String::from)
        .collect();

    Ok(TmuxLayout {
        session_name: session_name.to_string(),
        leader_pane: panes.first().cloned().unwrap_or_default(),
        worker_panes: panes.into_iter().skip(1).collect(),
    })
}

pub fn send_command_to_pane(session: &str, pane: &str, command: &str) -> Result<()> {
    Command::new("tmux")
        .args([
            "send-keys",
            "-t",
            &format!("{}:{}", session, pane),
            command,
            "Enter",
        ])
        .output()?;
    Ok(())
}

pub fn attach_session(session_name: &str) -> Result<()> {
    Command::new("tmux")
        .args(["attach-session", "-t", session_name])
        .status()?;
    Ok(())
}

pub fn kill_session(session_name: &str) -> Result<()> {
    Command::new("tmux")
        .args(["kill-session", "-t", session_name])
        .output()?;
    Ok(())
}
```

`crates/legion-core/src/squad/orchestrator.rs`:
```rust
use anyhow::Result;
use super::tmux::{create_squad_layout, send_command_to_pane, attach_session, TmuxLayout};

pub struct SquadOrchestrator {
    layout: Option<TmuxLayout>,
    worker_count: u32,
    base_port: u16,
}

impl SquadOrchestrator {
    pub fn new(worker_count: u32, base_port: u16) -> Self {
        Self {
            layout: None,
            worker_count,
            base_port,
        }
    }

    pub fn start(&mut self) -> Result<()> {
        let session_name = format!("legion-squad-{}", std::process::id());

        // Create tmux layout
        let layout = create_squad_layout(&session_name, self.worker_count)?;

        // Start daemon first (in background)
        // TODO: Start daemon process

        // Start leader
        let leader_port = self.base_port;
        let leader_cmd = format!("legion start --role leader --port {}", leader_port);
        send_command_to_pane(&session_name, &layout.leader_pane, &leader_cmd)?;

        // Start workers
        for (i, pane) in layout.worker_panes.iter().enumerate() {
            let worker_port = self.base_port + 1 + i as u16;
            let worker_cmd = format!(
                "legion start --role worker --id worker-{} --port {}",
                i + 1,
                worker_port
            );
            send_command_to_pane(&session_name, pane, &worker_cmd)?;
        }

        self.layout = Some(layout);

        // Attach to session
        attach_session(&session_name)?;

        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        if let Some(layout) = &self.layout {
            super::tmux::kill_session(&layout.session_name)?;
        }
        Ok(())
    }
}
```

**Step 2: Update CLI for squad mode**

`crates/legion-cli/src/main.rs`:
```rust
use anyhow::Result;
use clap::{Parser, Subcommand};
use legion_core::squad::SquadOrchestrator;

#[derive(Parser)]
#[command(name = "legion")]
#[command(about = "Claude Code wrapper with model switching and squad mode")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start legion in single window mode
    Start {
        #[arg(long)]
        role: Option<String>,

        #[arg(long)]
        id: Option<String>,

        #[arg(long, default_value = "18080")]
        port: u16,
    },
    /// Start legion in squad mode
    Squad {
        #[arg(short, long, default_value = "3")]
        workers: u32,

        #[arg(long, default_value = "18080")]
        base_port: u16,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Start { role, id, port }) => {
            match role.as_deref() {
                Some("leader") => {
                    println!("Starting Legion as leader on port {}...", port);
                    legion_tui::run_with_port(port).await?;
                }
                Some("worker") => {
                    let worker_id = id.unwrap_or_else(|| "worker-1".to_string());
                    println!("Starting Legion as worker {} on port {}...", worker_id, port);
                    legion_tui::run_with_port(port).await?;
                }
                _ => {
                    legion_tui::run_with_port(port).await?;
                }
            }
        }
        Some(Commands::Squad { workers, base_port }) => {
            println!("Starting Legion squad mode with {} workers...", workers);
            let mut orchestrator = SquadOrchestrator::new(workers, base_port);
            orchestrator.start()?;
        }
        None => {
            legion_tui::run().await?;
        }
    }

    Ok(())
}
```

**Step 3: Update legion-core lib.rs**

`crates/legion-core/src/lib.rs`:
```rust
pub mod proxy;
pub mod session;
pub mod ipc;
pub mod squad;

pub use proxy::ProxyServer;
```

**Step 4: Verify build**

Run: `cargo build`
Expected: Build succeeds

**Step 5: Commit**

```bash
git add .
git commit -m "feat(squad): add tmux orchestration for squad mode"
```

---

## Summary

This plan covers:

| Phase | Tasks | Description |
|-------|-------|-------------|
| 1 | 1-3 | Project skeleton, database, basic TUI |
| 2 | 4-5 | HTTP proxy with format transform |
| 3 | 6 | PTY embedding for Claude Code |
| 4 | 7 | Session discovery and management |
| 5 | 8-10 | Squad mode (IPC, daemon, orchestration) |

Each task is a small, testable unit with explicit file paths and code.
