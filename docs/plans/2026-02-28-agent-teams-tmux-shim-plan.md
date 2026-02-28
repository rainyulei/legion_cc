# Agent Teams tmux Shim Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Integrate Claude Code's native Agent Teams into Legion TUI via a fake tmux binary that intercepts pane creation and routes each teammate through a per-teammate proxy port.

**Architecture:** A `legion-tmux-shim` binary intercepts `tmux split-window` / `send-keys` / `list-panes` from Claude Code, communicates over a Unix domain socket to a Shim Controller inside `legion-core`, which emits events consumed by the TUI event loop to create teammate PTY panes with individual proxy routing.

**Tech Stack:** Rust, Unix domain sockets (`std::os::unix::net`), serde_json, tokio, ratatui, portable-pty, rusqlite

**Design doc:** `docs/plans/2026-02-28-agent-teams-tmux-shim-design.md`

---

### Task 1: IPC Protocol Types

**Files:**
- Create: `crates/legion-core/src/shim/mod.rs`
- Create: `crates/legion-core/src/shim/protocol.rs`
- Modify: `crates/legion-core/src/lib.rs:1` — add `pub mod shim;`

**Step 1: Add shim module to legion-core**

In `crates/legion-core/src/lib.rs`, add after line 6 (`pub mod orchestrate;`):

```rust
pub mod shim;
```

**Step 2: Create `crates/legion-core/src/shim/mod.rs`**

```rust
pub mod protocol;
```

**Step 3: Create `crates/legion-core/src/shim/protocol.rs`**

```rust
use serde::{Deserialize, Serialize};

/// Request from tmux shim binary → Legion shim controller
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
pub enum ShimRequest {
    SplitWindow {
        #[serde(default)]
        horizontal: bool,
    },
    SendKeys {
        target: String,
        keys: Vec<String>,
    },
    ListPanes {
        #[serde(default)]
        format: Option<String>,
    },
    DisplayMessage {
        #[serde(default)]
        format: Option<String>,
    },
    CapturePan {
        target: String,
    },
    HasSession {
        target: String,
    },
    KillPane {
        target: String,
    },
    Version,
    ListSessions,
}

/// Response from Legion shim controller → tmux shim binary
#[derive(Debug, Serialize, Deserialize)]
pub struct ShimResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pane_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ShimResponse {
    pub fn ok() -> Self {
        Self { success: true, output: None, pane_id: None, pane_index: None, error: None }
    }

    pub fn with_output(output: String) -> Self {
        Self { success: true, output: Some(output), pane_id: None, pane_index: None, error: None }
    }

    pub fn pane_created(pane_id: String, pane_index: usize) -> Self {
        Self { success: true, output: None, pane_id: Some(pane_id), pane_index: Some(pane_index), error: None }
    }

    pub fn fail(error: String) -> Self {
        Self { success: false, output: None, pane_id: None, pane_index: None, error: Some(error) }
    }
}
```

**Step 4: Verify**

Run: `cargo check -p legion-core`
Expected: compiles clean

**Step 5: Commit**

```bash
git add crates/legion-core/src/shim/ crates/legion-core/src/lib.rs
git commit -m "feat(core): add tmux shim IPC protocol types"
```

---

### Task 2: Shim Controller — Unix Socket Server

**Files:**
- Create: `crates/legion-core/src/shim/controller.rs`
- Modify: `crates/legion-core/src/shim/mod.rs` — add `pub mod controller;`
- Modify: `crates/legion-core/src/lib.rs` — re-export `ShimEvent`

**Step 1: Create `crates/legion-core/src/shim/controller.rs`**

```rust
use std::path::{Path, PathBuf};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use super::protocol::{ShimRequest, ShimResponse};

/// Events emitted by the shim controller, consumed by the TUI event loop.
#[derive(Debug)]
pub enum ShimEvent {
    /// `tmux split-window` — request a new pane. Reply with (pane_id, pane_index).
    PaneRequested {
        reply: tokio::sync::oneshot::Sender<(String, usize)>,
    },
    /// `tmux send-keys` — send a command string to a pane.
    SendKeys {
        pane_id: String,
        command: String,
    },
    /// `tmux list-panes` — reply with formatted pane list.
    ListPanes {
        format: Option<String>,
        reply: tokio::sync::oneshot::Sender<String>,
    },
    /// `tmux capture-pane` — reply with pane content.
    CapturePan {
        pane_id: String,
        reply: tokio::sync::oneshot::Sender<String>,
    },
}

/// Listens on a Unix domain socket for tmux shim requests and emits ShimEvents.
pub struct ShimController {
    socket_path: PathBuf,
    event_tx: mpsc::UnboundedSender<ShimEvent>,
}

impl ShimController {
    pub fn new(socket_path: PathBuf, event_tx: mpsc::UnboundedSender<ShimEvent>) -> Self {
        Self { socket_path, event_tx }
    }

    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Start accepting connections. Each connection handles one tmux command.
    pub async fn run(&self) -> anyhow::Result<()> {
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path)?;
        }
        if let Some(parent) = self.socket_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let listener = UnixListener::bind(&self.socket_path)?;
        info!("Shim controller listening on {:?}", self.socket_path);

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let tx = self.event_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, tx).await {
                            warn!("Shim connection error: {}", e);
                        }
                    });
                }
                Err(e) => error!("Shim accept error: {}", e),
            }
        }
    }
}

impl Drop for ShimController {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

async fn handle_connection(
    stream: tokio::net::UnixStream,
    event_tx: mpsc::UnboundedSender<ShimEvent>,
) -> anyhow::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut buf_reader = BufReader::new(reader);
    let mut line = String::new();
    buf_reader.read_line(&mut line).await?;

    let request: ShimRequest = serde_json::from_str(line.trim())?;
    let response = dispatch(request, &event_tx).await;
    let json = serde_json::to_string(&response)?;
    writer.write_all(json.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.shutdown().await?;
    Ok(())
}

async fn dispatch(
    request: ShimRequest,
    event_tx: &mpsc::UnboundedSender<ShimEvent>,
) -> ShimResponse {
    match request {
        ShimRequest::SplitWindow { .. } => {
            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            if event_tx.send(ShimEvent::PaneRequested { reply: reply_tx }).is_err() {
                return ShimResponse::fail("Legion not running".into());
            }
            match reply_rx.await {
                Ok((pane_id, pane_index)) => ShimResponse::pane_created(pane_id, pane_index),
                Err(_) => ShimResponse::fail("Pane creation timed out".into()),
            }
        }
        ShimRequest::SendKeys { target, keys } => {
            let command = keys.iter()
                .filter(|k| *k != "Enter" && *k != "C-c")
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            let _ = event_tx.send(ShimEvent::SendKeys { pane_id: target, command });
            ShimResponse::ok()
        }
        ShimRequest::ListPanes { format } => {
            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            if event_tx.send(ShimEvent::ListPanes { format, reply: reply_tx }).is_err() {
                return ShimResponse::with_output(String::new());
            }
            reply_rx.await.map(ShimResponse::with_output)
                .unwrap_or_else(|_| ShimResponse::with_output(String::new()))
        }
        ShimRequest::CapturePan { target } => {
            let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
            if event_tx.send(ShimEvent::CapturePan { pane_id: target, reply: reply_tx }).is_err() {
                return ShimResponse::with_output(String::new());
            }
            reply_rx.await.map(ShimResponse::with_output)
                .unwrap_or_else(|_| ShimResponse::with_output(String::new()))
        }
        ShimRequest::Version => ShimResponse::with_output("tmux 3.4".into()),
        ShimRequest::HasSession { .. } => ShimResponse::ok(),
        ShimRequest::ListSessions => ShimResponse::with_output(
            "legion: 1 windows (created Thu Jan  1 00:00:00 2026)".into()
        ),
        ShimRequest::DisplayMessage { format } => {
            let output = match format.as_deref() {
                Some(f) if f.contains("session_name") => "legion".to_string(),
                Some(f) if f.contains("window_index") => "0".to_string(),
                Some(f) if f.contains("pane_index") => "0".to_string(),
                _ => String::new(),
            };
            ShimResponse::with_output(output)
        }
        ShimRequest::KillPane { .. } => ShimResponse::ok(),
    }
}
```

**Step 2: Update `crates/legion-core/src/shim/mod.rs`**

```rust
pub mod controller;
pub mod protocol;
```

**Step 3: Re-export in `crates/legion-core/src/lib.rs`**

After existing `pub use` lines, add:

```rust
pub use shim::controller::ShimEvent;
```

**Step 4: Verify**

Run: `cargo check -p legion-core`

**Step 5: Commit**

```bash
git add crates/legion-core/src/shim/ crates/legion-core/src/lib.rs
git commit -m "feat(core): add shim controller with Unix socket server"
```

---

### Task 3: legion-tmux-shim Binary

**Files:**
- Create: `crates/legion-tmux-shim/Cargo.toml`
- Create: `crates/legion-tmux-shim/src/main.rs`
- Modify: `Cargo.toml:3` — add to workspace members
- Modify: `Makefile:15` — add to BINARIES
- Modify: `build-mac.sh:41` — add to binary loop

**Step 1: Create `crates/legion-tmux-shim/Cargo.toml`**

```toml
[package]
name = "legion-tmux-shim"
version.workspace = true
edition.workspace = true

[[bin]]
name = "legion-tmux-shim"
path = "src/main.rs"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
```

**Step 2: Create `crates/legion-tmux-shim/src/main.rs`**

```rust
//! Fake tmux binary — intercepts Claude Code Agent Teams commands
//! and forwards them to Legion's shim controller via Unix domain socket.

use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process;

use serde::{Deserialize, Serialize};

#[derive(Serialize)]
#[serde(tag = "command", rename_all = "kebab-case")]
enum Request {
    SplitWindow { horizontal: bool },
    SendKeys { target: String, keys: Vec<String> },
    ListPanes { format: Option<String> },
    DisplayMessage { format: Option<String> },
    CapturePan { target: String },
    HasSession { target: String },
    KillPane { target: String },
    Version,
    ListSessions,
}

#[derive(Deserialize)]
struct Response {
    #[allow(dead_code)]
    success: bool,
    output: Option<String>,
    #[allow(dead_code)]
    pane_id: Option<String>,
    #[allow(dead_code)]
    pane_index: Option<usize>,
    #[allow(dead_code)]
    error: Option<String>,
}

fn send_request(request: &Request) -> Option<Response> {
    let socket_path = env::var("LEGION_TMUX_SOCKET").ok()?;
    let mut stream = UnixStream::connect(&socket_path).ok()?;
    let json = serde_json::to_string(request).ok()?;
    stream.write_all(json.as_bytes()).ok()?;
    stream.write_all(b"\n").ok()?;
    stream.flush().ok()?;
    stream.shutdown(std::net::Shutdown::Write).ok()?;
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    serde_json::from_str(line.trim()).ok()
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() {
        eprintln!("legion-tmux-shim: no command specified");
        process::exit(1);
    }

    match args[0].as_str() {
        "-V" => {
            let out = send_request(&Request::Version)
                .and_then(|r| r.output)
                .unwrap_or_else(|| "tmux 3.4".into());
            println!("{}", out);
        }
        "split-window" | "splitw" => {
            let horizontal = args.iter().any(|a| a == "-h");
            send_request(&Request::SplitWindow { horizontal });
        }
        "send-keys" | "send" => {
            let (target, keys) = parse_send_keys(&args[1..]);
            send_request(&Request::SendKeys { target, keys });
        }
        "list-panes" | "lsp" => {
            let format = extract_flag_value(&args, "-F");
            if let Some(out) = send_request(&Request::ListPanes { format }).and_then(|r| r.output) {
                print!("{}", out);
            }
        }
        "display-message" | "display" => {
            let format = extract_flag_value(&args, "-p")
                .or_else(|| extract_flag_value(&args, "-F"));
            if let Some(out) = send_request(&Request::DisplayMessage { format }).and_then(|r| r.output) {
                println!("{}", out);
            }
        }
        "capture-pane" | "capturep" => {
            let target = extract_flag_value(&args, "-t").unwrap_or_default();
            if let Some(out) = send_request(&Request::CapturePan { target }).and_then(|r| r.output) {
                print!("{}", out);
            }
        }
        "has-session" | "has" => {
            let target = extract_flag_value(&args, "-t").unwrap_or_default();
            let ok = send_request(&Request::HasSession { target }).map(|r| r.success).unwrap_or(true);
            process::exit(if ok { 0 } else { 1 });
        }
        "list-sessions" | "ls" => {
            if let Some(out) = send_request(&Request::ListSessions).and_then(|r| r.output) {
                println!("{}", out);
            }
        }
        "kill-pane" | "killp" => {
            let target = extract_flag_value(&args, "-t").unwrap_or_default();
            send_request(&Request::KillPane { target });
        }
        _ => {
            eprintln!("legion-tmux-shim: unhandled: {} {:?}", args[0], &args[1..]);
        }
    }
}

fn parse_send_keys(args: &[String]) -> (String, Vec<String>) {
    let mut target = String::new();
    let mut keys = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-t" && i + 1 < args.len() {
            target = args[i + 1].clone();
            i += 2;
        } else {
            keys.push(args[i].clone());
            i += 1;
        }
    }
    (target, keys)
}

fn extract_flag_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find_map(|w| {
        if w[0] == flag { Some(w[1].clone()) } else { None }
    })
}
```

**Step 3: Add to workspace `Cargo.toml:3-10`**

Add `"crates/legion-tmux-shim"` to members list.

**Step 4: Add to `Makefile:15` BINARIES**

```makefile
BINARIES := legion legion-dispatch legion-check legion-report legion-status legion-stop legion-deps legion-tmux-shim
```

**Step 5: Add to `build-mac.sh:41` for loop**

```bash
for bin in legion legion-dispatch legion-check legion-report legion-status legion-stop legion-deps legion-tmux-shim; do
```

**Step 6: Verify**

Run: `cargo build -p legion-tmux-shim`

**Step 7: Commit**

```bash
git add crates/legion-tmux-shim/ Cargo.toml Makefile build-mac.sh
git commit -m "feat: add legion-tmux-shim binary crate"
```

---

### Task 4: DB Schema — Team Role Proxy Configs

**Files:**
- Modify: `crates/legion-db/src/schema.rs:127-194` — add `team_role_configs` table and session columns
- Modify: `crates/legion-db/src/repo.rs` — add CRUD and update SquadSession struct

**Step 1: Add table in `init_global_db()` before `Ok(())`**

```rust
    conn.execute_batch("CREATE TABLE IF NOT EXISTS team_role_configs (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        team_id TEXT NOT NULL,
        role_name TEXT NOT NULL,
        provider_id TEXT,
        model TEXT,
        created_at INTEGER NOT NULL,
        UNIQUE(team_id, role_name)
    );")?;
```

**Step 2: Add migrations in `init_project_db()` before `Ok(())`**

```rust
    let _ = conn.execute("ALTER TABLE squad_sessions ADD COLUMN enable_agent_teams INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE squad_sessions ADD COLUMN active_team_id TEXT", []);
```

**Step 3: Add `TeamRoleConfig` struct and repo methods in `repo.rs`**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamRoleConfig {
    pub team_id: String,
    pub role_name: String,
    pub provider_id: Option<String>,
    pub model: Option<String>,
}
```

Add to `impl Repository`:

```rust
    pub fn upsert_team_role_config(&self, config: &TeamRoleConfig) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
        self.conn.execute(
            "INSERT INTO team_role_configs (team_id, role_name, provider_id, model, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(team_id, role_name) DO UPDATE SET provider_id=?3, model=?4",
            params![config.team_id, config.role_name, config.provider_id, config.model, now],
        )?;
        Ok(())
    }

    pub fn list_team_role_configs(&self, team_id: &str) -> Result<Vec<TeamRoleConfig>> {
        let mut stmt = self.conn.prepare(
            "SELECT team_id, role_name, provider_id, model FROM team_role_configs WHERE team_id = ?1"
        )?;
        let rows = stmt.query_map(params![team_id], |row| {
            Ok(TeamRoleConfig {
                team_id: row.get(0)?,
                role_name: row.get(1)?,
                provider_id: row.get(2)?,
                model: row.get(3)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn delete_team_role_configs(&self, team_id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM team_role_configs WHERE team_id = ?1", params![team_id])?;
        Ok(())
    }
```

**Step 4: Add fields to `SquadSession` struct in `repo.rs:26-38`**

```rust
    pub enable_agent_teams: bool,
    pub active_team_id: Option<String>,
```

Update `SquadSession` loading queries to read these columns with defaults for older DBs.

**Step 5: Verify**

Run: `cargo check -p legion-db`

**Step 6: Commit**

```bash
git add crates/legion-db/
git commit -m "feat(db): add team_role_configs table and session agent teams flag"
```

---

### Task 5: Leader PTY Environment Setup

**Files:**
- Modify: `crates/legion-tui/src/pty.rs:28-40` — add `shim_socket_path` parameter
- Modify: `crates/legion-tui/src/pty.rs:68-91` — set TMUX, PATH, LEGION_TMUX_SOCKET
- Modify: `crates/legion-tui/src/app.rs:555-576` — pass socket path through `add_pane()`
- Modify: all `add_pane()` call sites — add `None` or socket path as last arg

**Step 1: Extend `PtyHandle::spawn()` signature (pty.rs:28)**

Add after `continue_session: bool`:

```rust
        shim_socket_path: Option<&std::path::Path>,
```

**Step 2: Add shim env block after PATH setup (pty.rs, after line 83)**

```rust
        // Fake tmux environment for Agent Teams integration (Leader only)
        if let Some(socket_path) = shim_socket_path {
            let shim_dir = std::env::temp_dir()
                .join(format!("legion-shim-{}", std::process::id()));
            std::fs::create_dir_all(&shim_dir).ok();
            if let Ok(self_exe) = std::env::current_exe() {
                let shim_bin = self_exe.with_file_name("legion-tmux-shim");
                if shim_bin.exists() {
                    let tmux_link = shim_dir.join("tmux");
                    let _ = std::fs::remove_file(&tmux_link);
                    #[cfg(unix)]
                    std::os::unix::fs::symlink(&shim_bin, &tmux_link).ok();
                }
            }
            // Prepend shim dir so our fake tmux is found first
            let current_path = std::env::var("PATH").unwrap_or_default();
            cmd.env("PATH", format!("{}:{}", shim_dir.display(), current_path));
            // Claude Code checks $TMUX to decide tmux mode
            cmd.env("TMUX", format!("/tmp/legion-tmux-fake/{},0,0", std::process::id()));
            cmd.env("LEGION_TMUX_SOCKET", socket_path.to_string_lossy().to_string());
        }
```

**Step 3: Add param to `add_pane()` and forward (app.rs:555)**

```rust
    pub fn add_pane(
        &mut self, rows: u16, cols: u16,
        proxy_port: u16, control_port: u16,
        label: String, dangerously_skip_permissions: bool,
        worker_id: Option<u16>, orchestrate_port: Option<u16>,
        system_prompt: Option<&str>, working_dir: Option<&std::path::Path>,
        continue_session: bool,
        shim_socket_path: Option<&std::path::Path>,  // NEW
    ) {
```

Forward to `PtyHandle::spawn(... shim_socket_path)`.

**Step 4: Update all call sites**

- `app.rs:1132` (start_session Leader) → `self.shim_socket_path.as_deref()`
- `app.rs` (check_continue_fallback) → `None`
- `lib.rs:50` (single-pane run) → `None`
- Any dynamic add_worker → `None`

**Step 5: Verify**

Run: `cargo check -p legion-tui`

**Step 6: Commit**

```bash
git add crates/legion-tui/src/pty.rs crates/legion-tui/src/app.rs crates/legion-tui/src/lib.rs
git commit -m "feat(tui): set up fake tmux env in Leader PTY for Agent Teams"
```

---

### Task 6: Teammate Pane Data Structures

**Files:**
- Modify: `crates/legion-tui/src/app.rs` — add TeammatePane struct + App fields + init

**Step 1: Add `TeammatePane` after Pane struct (after line 228)**

```rust
pub struct TeammatePane {
    pub pane_id: String,        // "%1", "%2", ...
    pub pane_index: usize,
    pub pty: Option<PtyHandle>,
    pub proxy_port: u16,
    pub control_port: u16,
    pub agent_name: Option<String>,
    pub agent_type: Option<String>,
    pub team_name: Option<String>,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub scroll_offset: usize,
}
```

**Step 2: Add fields to App (before closing `}` at line 423)**

```rust
    pub teammate_panes: Vec<TeammatePane>,
    pub shim_event_rx: Option<tokio::sync::mpsc::UnboundedReceiver<legion_core::ShimEvent>>,
    pub shim_socket_path: Option<PathBuf>,
    pub next_teammate_port: u16,
    pub agent_teams_enabled: bool,
    pub active_team_id: Option<String>,
```

**Step 3: Initialize in App::new()**

```rust
            teammate_panes: Vec::new(),
            shim_event_rx: None,
            shim_socket_path: None,
            next_teammate_port: 0,
            agent_teams_enabled: false,
            active_team_id: None,
```

**Step 4: Verify**

Run: `cargo check -p legion-tui`

**Step 5: Commit**

```bash
git add crates/legion-tui/src/app.rs
git commit -m "feat(tui): add TeammatePane struct and agent teams state to App"
```

---

### Task 7: Teammate Helper Functions

**Files:**
- Create: `crates/legion-tui/src/teammate.rs`
- Modify: `crates/legion-tui/src/lib.rs:10` — add `pub mod teammate;`

**Step 1: Create `crates/legion-tui/src/teammate.rs`**

```rust
//! Teammate PTY management — spawning, arg parsing, pane formatting, proxy resolution.

use std::path::Path;
use crate::app::TeammatePane;
use crate::pty::PtyHandle;
use legion_db::Provider;

pub struct ParsedClaude {
    pub agent_name: Option<String>,
    pub agent_type: Option<String>,
    pub team_name: Option<String>,
    pub model: Option<String>,
}

pub fn parse_claude_args(command: &str) -> ParsedClaude {
    let parts: Vec<&str> = command.split_whitespace().collect();
    let mut r = ParsedClaude { agent_name: None, agent_type: None, team_name: None, model: None };
    let mut i = 0;
    while i < parts.len() {
        match parts[i] {
            "--agent-name" if i + 1 < parts.len() => { r.agent_name = Some(parts[i+1].to_string()); i += 2; }
            "--agent-type" if i + 1 < parts.len() => { r.agent_type = Some(parts[i+1].to_string()); i += 2; }
            "--team-name" if i + 1 < parts.len() => { r.team_name = Some(parts[i+1].to_string()); i += 2; }
            "--model" if i + 1 < parts.len() => { r.model = Some(parts[i+1].to_string()); i += 2; }
            _ => i += 1,
        }
    }
    r
}

pub fn spawn_teammate_pty(
    proxy_port: u16, control_port: u16,
    working_dir: Option<&Path>, rows: u16, cols: u16,
) -> Option<PtyHandle> {
    match PtyHandle::spawn(
        rows, cols, proxy_port, control_port,
        true, None, None, None, true, working_dir, false, None,
    ) {
        Ok(pty) => Some(pty),
        Err(e) => { tracing::error!("Failed to spawn teammate PTY: {}", e); None }
    }
}

pub fn resolve_teammate_proxy(
    agent_name: Option<&str>, agent_type: Option<&str>,
    team_id: Option<&str>,
    leader_provider_id: Option<&str>, leader_model: Option<&str>,
) -> (Option<String>, Option<String>) {
    if let Some(tid) = team_id {
        if let Ok(repo) = legion_db::open_db() {
            if let Ok(configs) = repo.list_team_role_configs(tid) {
                if let Some(name) = agent_name {
                    if let Some(c) = configs.iter().find(|c| c.role_name == name) {
                        if c.provider_id.is_some() { return (c.provider_id.clone(), c.model.clone()); }
                    }
                }
                if let Some(atype) = agent_type {
                    let role = match atype {
                        "code-reviewer" | "tech-lead" => "tech_lead",
                        "general-purpose" | "engineer" => "engineer",
                        "qa" | "tester" => "qa",
                        _ => atype,
                    };
                    if let Some(c) = configs.iter().find(|c| c.role_name == role) {
                        if c.provider_id.is_some() { return (c.provider_id.clone(), c.model.clone()); }
                    }
                }
            }
        }
    }
    (leader_provider_id.map(|s| s.to_string()), leader_model.map(|s| s.to_string()))
}

pub fn format_pane_list(panes: &[TeammatePane], format: Option<&str>) -> String {
    let fmt = format.unwrap_or("#{pane_index}");
    let mut out = String::new();
    // Leader = pane %0
    let leader = fmt.replace("#{pane_index}", "0").replace("#{pane_id}", "%0")
        .replace("#{pane_width}", "120").replace("#{pane_height}", "40").replace("#{pane_active}", "1");
    out.push_str(&leader); out.push('\n');
    for p in panes {
        let line = fmt.replace("#{pane_index}", &p.pane_index.to_string())
            .replace("#{pane_id}", &p.pane_id)
            .replace("#{pane_width}", "60").replace("#{pane_height}", "20").replace("#{pane_active}", "0");
        out.push_str(&line); out.push('\n');
    }
    out
}

pub fn capture_pane_content(panes: &[TeammatePane], pane_id: &str) -> String {
    let pane = match panes.iter().find(|p| p.pane_id == pane_id) { Some(p) => p, None => return String::new() };
    let pty = match pane.pty.as_ref() { Some(p) => p, None => return String::new() };
    if let Ok(parser) = pty.parser.lock() {
        let screen = parser.screen();
        let (rows, cols) = screen.size();
        screen.rows(0, cols).take(rows as usize).collect::<Vec<_>>().join("\n")
    } else { String::new() }
}
```

**Step 2: Add module in lib.rs after `pub mod worktree;`**

```rust
pub mod teammate;
```

**Step 3: Verify**

Run: `cargo check -p legion-tui`

**Step 4: Commit**

```bash
git add crates/legion-tui/src/teammate.rs crates/legion-tui/src/lib.rs
git commit -m "feat(tui): add teammate helper functions"
```

---

### Task 8: Shim Event Polling in Event Loop

**Files:**
- Modify: `crates/legion-tui/src/lib.rs:174-360` — poll shim events, create teammate panes

**Step 1: Add shim event polling block**

In `run_event_loop()`, after the copilot auth check (around line 315), before `terminal.draw(...)`:

```rust
        // Poll tmux shim events (Agent Teams)
        if let Some(ref mut rx) = app.shim_event_rx {
            while let Ok(event) = rx.try_recv() {
                match event {
                    legion_core::ShimEvent::PaneRequested { reply } => {
                        let idx = app.teammate_panes.len();
                        let pane_id = format!("%{}", idx + 1);
                        let proxy_port = app.next_teammate_port;
                        let control_port = proxy_port + 1000;
                        app.next_teammate_port += 1;
                        app.teammate_panes.push(app::TeammatePane {
                            pane_id: pane_id.clone(), pane_index: idx + 1,
                            pty: None, proxy_port, control_port,
                            agent_name: None, agent_type: None, team_name: None,
                            provider_id: None, model: None, scroll_offset: 0,
                        });
                        let _ = reply.send((pane_id, idx + 1));
                    }
                    legion_core::ShimEvent::SendKeys { pane_id, command } => {
                        if let Some(tm) = app.teammate_panes.iter_mut().find(|t| t.pane_id == pane_id) {
                            let parsed = teammate::parse_claude_args(&command);
                            tm.agent_name = parsed.agent_name;
                            tm.agent_type = parsed.agent_type;
                            tm.team_name = parsed.team_name;
                            let leader_pid = app.current_provider.and_then(|i| app.providers.get(i)).map(|p| p.id.as_str());
                            let (pid, model) = teammate::resolve_teammate_proxy(
                                tm.agent_name.as_deref(), tm.agent_type.as_deref(),
                                app.active_team_id.as_deref(), leader_pid, app.current_model.as_deref(),
                            );
                            tm.provider_id = pid;
                            tm.model = parsed.model.or(model);
                            let (_, th) = app.term_size;
                            let rows = th.saturating_sub(6);
                            let cols = 60u16;
                            start_worker_proxy(tm.proxy_port, tm.control_port,
                                &tm.agent_name.clone().unwrap_or_else(|| pane_id.clone())).await;
                            // TODO: send proxy config for this teammate port
                            tm.pty = teammate::spawn_teammate_pty(
                                tm.proxy_port, tm.control_port,
                                app.project_path.as_deref(), rows, cols,
                            );
                        }
                    }
                    legion_core::ShimEvent::ListPanes { format, reply } => {
                        let out = teammate::format_pane_list(&app.teammate_panes, format.as_deref());
                        let _ = reply.send(out);
                    }
                    legion_core::ShimEvent::CapturePan { pane_id, reply } => {
                        let out = teammate::capture_pane_content(&app.teammate_panes, &pane_id);
                        let _ = reply.send(out);
                    }
                }
            }
        }
```

**Step 2: Verify**

Run: `cargo check -p legion-tui`

**Step 3: Commit**

```bash
git add crates/legion-tui/src/lib.rs
git commit -m "feat(tui): poll shim events and create teammate PTYs in event loop"
```

---

### Task 9: Start Shim Controller on Session Start

**Files:**
- Modify: `crates/legion-tui/src/app.rs:1062-1197` — init shim controller + pass socket to Leader

**Step 1: Add shim init in `start_session()` after line 1127 (before port assignments)**

```rust
        // Start shim controller for Agent Teams
        let socket_path = std::env::temp_dir()
            .join(format!("legion-shim-{}.sock", std::process::id()));
        self.shim_socket_path = Some(socket_path.clone());
        self.next_teammate_port = self.base_port + 3000;
        self.teammate_panes.clear();
        let (shim_tx, shim_rx) = tokio::sync::mpsc::unbounded_channel();
        self.shim_event_rx = Some(shim_rx);
        let controller = legion_core::shim::controller::ShimController::new(socket_path, shim_tx);
        tokio::spawn(async move {
            if let Err(e) = controller.run().await {
                tracing::error!("Shim controller error: {}", e);
            }
        });
```

**Step 2: Pass socket to Leader's add_pane (line ~1132)**

Change last arg from `None` to `self.shim_socket_path.as_deref()`.

**Step 3: Add cleanup in `kill_all()` (after line 617)**

```rust
        for tm in &mut self.teammate_panes {
            if let Some(ref mut pty) = tm.pty { pty.kill(); }
        }
        self.teammate_panes.clear();
        if let Some(ref path) = self.shim_socket_path {
            let _ = std::fs::remove_file(path);
        }
        let shim_dir = std::env::temp_dir().join(format!("legion-shim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&shim_dir);
```

**Step 4: Verify**

Run: `cargo check -p legion-tui`

**Step 5: Commit**

```bash
git add crates/legion-tui/src/app.rs
git commit -m "feat(tui): start shim controller on session start, cleanup on exit"
```

---

### Task 10: Three-Column TUI Layout

**Files:**
- Modify: `crates/legion-tui/src/ui.rs:188-207` — conditional three-column layout
- Modify: `crates/legion-tui/src/ui.rs` — add `draw_teammate_column()`, `draw_teammate_pane()`

**Step 1: Replace `draw_squad_layout` (line 188-207)**

```rust
fn draw_squad_layout(frame: &mut Frame, app: &mut App, area: Rect) {
    if !app.teammate_panes.is_empty() {
        let leader_pct = 40u16.min(app.leader_ratio);
        let leader_w = (area.width as u32 * leader_pct as u32 / 100) as u16;
        let board_w = 30u16.min(area.width / 4);
        let teammate_w = area.width.saturating_sub(leader_w + board_w + 2);
        let h = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(leader_w), Constraint::Length(1),
                Constraint::Length(teammate_w), Constraint::Length(1),
                Constraint::Length(board_w),
            ]).split(area);
        draw_pane(frame, app, 0, h[0]);
        draw_divider(frame, app, h[1]);
        draw_teammate_column(frame, app, h[2]);
        draw_divider(frame, app, h[3]);
        draw_task_board(frame, app, h[4]);
    } else {
        let leader_width = (area.width as u32 * app.leader_ratio as u32 / 100) as u16;
        let h = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(leader_width), Constraint::Length(1), Constraint::Min(0),
            ]).split(area);
        draw_pane(frame, app, 0, h[0]);
        draw_divider(frame, app, h[1]);
        draw_task_board(frame, app, h[2]);
    }
}
```

**Step 2: Add teammate rendering (after `draw_divider`)**

```rust
fn draw_teammate_column(frame: &mut Frame, app: &mut App, area: Rect) {
    let count = app.teammate_panes.len();
    if count == 0 { return; }
    let constraints: Vec<Constraint> = (0..count).map(|_| Constraint::Ratio(1, count as u32)).collect();
    let chunks = Layout::default().direction(Direction::Vertical).constraints(constraints).split(area);
    for (i, chunk) in chunks.iter().enumerate() {
        draw_teammate_pane(frame, app, i, *chunk);
    }
}

fn draw_teammate_pane(frame: &mut Frame, app: &App, index: usize, area: Rect) {
    let tm = match app.teammate_panes.get(index) { Some(t) => t, None => return };
    let name = tm.agent_name.as_deref().unwrap_or("teammate");
    let model = tm.model.as_deref().unwrap_or("default");
    let title = format!(" {} [{}] ", name, model);
    let block = Block::default().title(title).borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if let Some(ref pty) = tm.pty {
        if let Ok(p) = pty.parser.lock() {
            let pseudo = PseudoTerminal::new(p.screen());
            frame.render_widget(pseudo, inner);
        }
    } else {
        frame.render_widget(
            Paragraph::new("  Waiting for teammate...").style(Style::default().fg(Color::DarkGray)),
            inner,
        );
    }
}
```

**Step 3: Show teammate count in header (after Workers span)**

```rust
    if !app.teammate_panes.is_empty() {
        spans.push(Span::styled(
            format!("  Teammates: {}", app.teammate_panes.len()),
            Style::default().fg(Color::Magenta),
        ));
    }
```

**Step 4: Verify**

Run: `cargo check -p legion-tui`

**Step 5: Commit**

```bash
git add crates/legion-tui/src/ui.rs
git commit -m "feat(tui): three-column layout with teammate panes"
```

---

### Task 11: Resize + Tab Cycling + Agent Teams Config UI

**Files:**
- Modify: `crates/legion-tui/src/app.rs:702-727` — resize teammate PTYs
- Modify: `crates/legion-tui/src/input.rs` — Tab includes teammates
- Modify: `crates/legion-tui/src/app.rs` — add `AgentTeamsConfig` popup + menu entry
- Modify: `crates/legion-tui/src/ui.rs` — render Agent Teams config popup
- Modify: `crates/legion-tui/src/input.rs` — handle Agent Teams config input

**Step 1: Resize teammates in `resize_panes()` after leader block**

```rust
            // Resize teammate PTYs
            if !self.teammate_panes.is_empty() {
                let tm_w = term_width.saturating_sub(leader_width + 30 + 2);
                let tm_cols = tm_w.saturating_sub(2);
                let tm_count = self.teammate_panes.len() as u16;
                let tm_rows = (content_height / tm_count).saturating_sub(2);
                for tm in &mut self.teammate_panes {
                    if let Some(ref mut pty) = tm.pty { let _ = pty.resize(tm_rows, tm_cols); }
                }
            }
```

**Step 2: Add `AgentTeamsConfig` PopupMenu variant and MainMenuItem**

In `PopupMenu` enum add `AgentTeamsConfig`. In `MainMenuItem` add `AgentTeams` with label "Agent Teams" and description "Configure Agent Teams proxy routing per role".

**Step 3: Simple config popup rendering + input handling**

Render a popup showing:
- Enable toggle
- Active team name
- Role → provider/model list
- Enter to edit, Esc to close

Use existing popup rendering patterns from the codebase.

**Step 4: Full build**

Run: `cargo build --release`

**Step 5: Commit**

```bash
git add crates/legion-tui/
git commit -m "feat: resize, tab cycling, Agent Teams config UI"
```

---

### Task 12: Final Integration + Manual Test

**Step 1: Full build + install**

Run: `cargo build --release && make install`

**Step 2: Manual test checklist**

- [ ] `legion-tmux-shim` binary exists alongside `legion`
- [ ] Start session → Leader PTY has `TMUX` and `LEGION_TMUX_SOCKET` env vars
- [ ] `LEGION_TMUX_SOCKET` points to a valid Unix socket
- [ ] Shim controller log shows "listening on ..."
- [ ] Agent Teams in Leader creates teammate panes in middle column
- [ ] Each teammate has separate proxy port
- [ ] Ctrl+P → Agent Teams shows config popup
- [ ] Ctrl+Q cleans up socket and shim dir
- [ ] Without Agent Teams, layout is unchanged (two-column)

**Step 3: Commit final polish**

```bash
git add -A
git commit -m "feat: complete Agent Teams tmux shim integration"
```

---

## Dependency Graph

```
Task 1 (protocol) ──→ Task 2 (controller) ──→ Task 3 (shim binary)
                                                      ↓
Task 4 (DB schema)        Task 6 (data structs) ← Task 5 (leader env)
      ↓                          ↓
      └──→ Task 7 (helpers) ←────┘
                 ↓
           Task 8 (event loop)
                 ↓
           Task 9 (session start)
                 ↓
           Task 10 (three-col layout)
                 ↓
           Task 11 (resize + config UI)
                 ↓
           Task 12 (integration test)
```

**Critical path:** 1 → 2 → 5 → 6 → 7 → 8 → 9 → 10 → 12
