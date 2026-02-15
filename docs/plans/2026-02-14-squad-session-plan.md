# Squad Session Management Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Squad mode supports persistent sessions with git worktrees, session switching, and task completion with merge strategy selection.

**Architecture:** Each session creates isolated git worktrees per pane (Leader + Workers). Sessions are tracked in SQLite. On resume, `claude --continue` runs in each worktree directory to restore context. Ctrl+P menu gains session management items alongside existing model switching.

**Tech Stack:** Rust, ratatui, SQLite (rusqlite), git CLI (std::process::Command), portable-pty

---

### Task 1: DB Schema + SquadSession Model

Add the `squad_sessions` table and CRUD methods to the DB layer.

**Files:**
- Modify: `crates/legion-db/src/schema.rs:1-62`
- Modify: `crates/legion-db/src/repo.rs:1-324`
- Modify: `crates/legion-db/src/lib.rs:1-23`

**Step 1: Write the failing tests**

Add to `crates/legion-db/src/repo.rs` in the `#[cfg(test)] mod tests` block (after line 323):

```rust
#[test]
fn squad_session_crud() {
    let repo = test_repo();

    // Empty initially
    let all = repo.list_squad_sessions().unwrap();
    assert!(all.is_empty());

    // Insert
    let session = SquadSession {
        name: "fix-auth-bug".into(),
        project_path: "/home/user/my-app".into(),
        worker_count: 2,
        status: "active".into(),
        created_at: 1000,
        completed_at: None,
    };
    repo.upsert_squad_session(&session).unwrap();

    // Get
    let loaded = repo.get_squad_session("fix-auth-bug").unwrap().unwrap();
    assert_eq!(loaded.project_path, "/home/user/my-app");
    assert_eq!(loaded.worker_count, 2);
    assert_eq!(loaded.status, "active");
    assert!(loaded.completed_at.is_none());

    // List
    let all = repo.list_squad_sessions().unwrap();
    assert_eq!(all.len(), 1);

    // Update status
    repo.complete_squad_session("fix-auth-bug", 2000).unwrap();
    let loaded = repo.get_squad_session("fix-auth-bug").unwrap().unwrap();
    assert_eq!(loaded.status, "completed");
    assert_eq!(loaded.completed_at, Some(2000));

    // Delete
    repo.delete_squad_session("fix-auth-bug").unwrap();
    assert!(repo.get_squad_session("fix-auth-bug").unwrap().is_none());
}

#[test]
fn squad_session_list_active_only() {
    let repo = test_repo();

    repo.upsert_squad_session(&SquadSession {
        name: "active-one".into(),
        project_path: "/tmp".into(),
        worker_count: 1,
        status: "active".into(),
        created_at: 100,
        completed_at: None,
    }).unwrap();
    repo.upsert_squad_session(&SquadSession {
        name: "done-one".into(),
        project_path: "/tmp".into(),
        worker_count: 2,
        status: "completed".into(),
        created_at: 50,
        completed_at: Some(200),
    }).unwrap();

    let active = repo.list_active_squad_sessions().unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].name, "active-one");

    let all = repo.list_squad_sessions().unwrap();
    assert_eq!(all.len(), 2);
}
```

**Step 2: Run tests to verify they fail**

Run: `cargo test -p legion-db`
Expected: FAIL — `SquadSession` not defined, methods don't exist.

**Step 3: Add schema**

In `crates/legion-db/src/schema.rs`, add the `squad_sessions` table inside the `SCHEMA` string (before the closing `"#;`):

```sql
CREATE TABLE IF NOT EXISTS squad_sessions (
    name TEXT PRIMARY KEY,
    project_path TEXT NOT NULL,
    worker_count INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    created_at INTEGER NOT NULL,
    completed_at INTEGER
);
```

**Step 4: Add SquadSession struct and methods**

In `crates/legion-db/src/repo.rs`, add the struct after `PaneConfig` (after line 23):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SquadSession {
    pub name: String,
    pub project_path: String,
    pub worker_count: i64,
    pub status: String,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}
```

Add methods to `impl Repository` (before the closing `}`):

```rust
// Squad session methods

pub fn upsert_squad_session(&self, session: &SquadSession) -> Result<()> {
    self.conn.execute(
        "INSERT OR REPLACE INTO squad_sessions (name, project_path, worker_count, status, created_at, completed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![session.name, session.project_path, session.worker_count, session.status, session.created_at, session.completed_at],
    )?;
    Ok(())
}

pub fn get_squad_session(&self, name: &str) -> Result<Option<SquadSession>> {
    let mut stmt = self.conn.prepare(
        "SELECT name, project_path, worker_count, status, created_at, completed_at FROM squad_sessions WHERE name = ?"
    )?;
    let mut rows = stmt.query(params![name])?;
    if let Some(row) = rows.next()? {
        Ok(Some(SquadSession {
            name: row.get(0)?,
            project_path: row.get(1)?,
            worker_count: row.get(2)?,
            status: row.get(3)?,
            created_at: row.get(4)?,
            completed_at: row.get(5)?,
        }))
    } else {
        Ok(None)
    }
}

pub fn list_squad_sessions(&self) -> Result<Vec<SquadSession>> {
    let mut stmt = self.conn.prepare(
        "SELECT name, project_path, worker_count, status, created_at, completed_at FROM squad_sessions ORDER BY created_at DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(SquadSession {
            name: row.get(0)?,
            project_path: row.get(1)?,
            worker_count: row.get(2)?,
            status: row.get(3)?,
            created_at: row.get(4)?,
            completed_at: row.get(5)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn list_active_squad_sessions(&self) -> Result<Vec<SquadSession>> {
    let mut stmt = self.conn.prepare(
        "SELECT name, project_path, worker_count, status, created_at, completed_at FROM squad_sessions WHERE status = 'active' ORDER BY created_at DESC"
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(SquadSession {
            name: row.get(0)?,
            project_path: row.get(1)?,
            worker_count: row.get(2)?,
            status: row.get(3)?,
            created_at: row.get(4)?,
            completed_at: row.get(5)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn complete_squad_session(&self, name: &str, completed_at: i64) -> Result<()> {
    self.conn.execute(
        "UPDATE squad_sessions SET status = 'completed', completed_at = ?1 WHERE name = ?2",
        params![completed_at, name],
    )?;
    Ok(())
}

pub fn delete_squad_session(&self, name: &str) -> Result<()> {
    self.conn.execute("DELETE FROM squad_sessions WHERE name = ?", params![name])?;
    Ok(())
}
```

**Step 5: Update lib.rs exports**

In `crates/legion-db/src/lib.rs`, change line 8 to:

```rust
pub use repo::{PaneConfig, Provider, Repository, Session, SquadSession};
```

**Step 6: Run tests to verify they pass**

Run: `cargo test -p legion-db`
Expected: All tests PASS (including the 2 new tests).

**Step 7: Commit**

```bash
git add crates/legion-db/
git commit -m "feat(db): add squad_sessions table and CRUD methods"
```

---

### Task 2: PTY Spawn — Add working_dir and continue_session

Extend `PtyHandle::spawn()` and `App::add_pane()` to support setting a working directory and `--continue` flag.

**Files:**
- Modify: `crates/legion-tui/src/pty.rs:28-38` (spawn signature)
- Modify: `crates/legion-tui/src/app.rs:136-176` (add_pane signature)
- Modify: `crates/legion-tui/src/lib.rs:43,102-108` (callers)

**Step 1: Modify PtyHandle::spawn() signature and body**

In `crates/legion-tui/src/pty.rs`, change the `spawn` method signature (lines 28-38) to add two new parameters:

```rust
pub fn spawn(
    rows: u16,
    cols: u16,
    proxy_port: u16,
    control_port: u16,
    dangerously_skip_permissions: bool,
    worker_id: Option<u16>,
    orchestrate_port: Option<u16>,
    system_prompt: Option<&str>,
    use_proxy: bool,
    working_dir: Option<&std::path::Path>,    // NEW
    continue_session: bool,                     // NEW
) -> Result<Self> {
```

After the line `let mut cmd = CommandBuilder::new("claude");` (line 49), add working directory support:

```rust
if let Some(dir) = working_dir {
    cmd.cwd(dir);
}
if continue_session {
    cmd.arg("--continue");
}
```

**Step 2: Modify App::add_pane() signature**

In `crates/legion-tui/src/app.rs`, change `add_pane` (lines 136-176) to accept the new params:

```rust
pub fn add_pane(
    &mut self,
    rows: u16,
    cols: u16,
    proxy_port: u16,
    control_port: u16,
    label: String,
    dangerously_skip_permissions: bool,
    worker_id: Option<u16>,
    orchestrate_port: Option<u16>,
    system_prompt: Option<&str>,
    working_dir: Option<&std::path::Path>,    // NEW
    continue_session: bool,                     // NEW
) {
    let use_proxy = self.pane_uses_proxy(&label);
    let pty = match PtyHandle::spawn(rows, cols, proxy_port, control_port, dangerously_skip_permissions, worker_id, orchestrate_port, system_prompt, use_proxy, working_dir, continue_session) {
```

**Step 3: Update all callers in lib.rs**

In `crates/legion-tui/src/lib.rs`:

Line 43 (single pane `run()`), add `None, false`:
```rust
app.add_pane(pty_rows, pty_cols, proxy_port, control_port, "Claude Code".into(), false, None, None, None, None, false);
```

Line 102 (leader in `run_squad()`), add `None, false`:
```rust
app.add_pane(leader_pty_rows, leader_pty_cols, leader_proxy, leader_control, "Leader".into(), false, None, Some(orchestrate_port), Some(&leader_prompt), None, false);
```

Line 108 (workers in `run_squad()`), add `None, false`:
```rust
app.add_pane(worker_pty_rows, worker_pty_cols, proxy, control, label, true, Some(i + 1), Some(orchestrate_port), Some(&worker_prompts[i as usize]), None, false);
```

**Step 4: Compile and verify**

Run: `cargo build`
Expected: Compiles cleanly. Behavior unchanged (all new params are `None`/`false`).

**Step 5: Commit**

```bash
git add crates/legion-tui/
git commit -m "feat(pty): add working_dir and continue_session params to spawn"
```

---

### Task 3: Worktree Management Module

Create a new module that wraps git CLI commands for worktree create/verify/remove/merge operations.

**Files:**
- Create: `crates/legion-tui/src/worktree.rs`
- Modify: `crates/legion-tui/src/lib.rs:1-7` (add `pub mod worktree;`)

**Step 1: Create the worktree module**

Create `crates/legion-tui/src/worktree.rs`:

```rust
//! Git worktree management for squad sessions
//!
//! Each session creates isolated worktrees per pane:
//!   ../<project>-legion/<session>/<pane-label>/
//! Branch naming: legion/<session>/<pane-label>

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

/// Compute the worktree root for a project: <parent>/<project-name>-legion/
pub fn legion_root(project_path: &Path) -> PathBuf {
    let parent = project_path.parent().unwrap_or(project_path);
    let name = project_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("project");
    parent.join(format!("{}-legion", name))
}

/// Compute the worktree path for a specific pane in a session
pub fn pane_worktree_path(project_path: &Path, session_name: &str, pane_label: &str) -> PathBuf {
    let label_slug = pane_label.to_lowercase().replace(' ', "-");
    legion_root(project_path)
        .join(session_name)
        .join(label_slug)
}

/// Compute the git branch name for a pane
pub fn pane_branch_name(session_name: &str, pane_label: &str) -> String {
    let label_slug = pane_label.to_lowercase().replace(' ', "-");
    format!("legion/{}/{}", session_name, label_slug)
}

/// Create a git worktree for a pane. Runs from the project root.
pub fn create_worktree(
    project_path: &Path,
    session_name: &str,
    pane_label: &str,
) -> Result<PathBuf> {
    let wt_path = pane_worktree_path(project_path, session_name, pane_label);
    let branch = pane_branch_name(session_name, pane_label);

    // Ensure parent directory exists
    if let Some(parent) = wt_path.parent() {
        std::fs::create_dir_all(parent)
            .context("Failed to create worktree parent directory")?;
    }

    let output = Command::new("git")
        .args(["worktree", "add", &wt_path.to_string_lossy(), "-b", &branch])
        .current_dir(project_path)
        .output()
        .context("Failed to run git worktree add")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git worktree add failed: {}", stderr.trim());
    }

    Ok(wt_path)
}

/// Check if a worktree path exists and is valid
pub fn worktree_exists(path: &Path) -> bool {
    path.is_dir() && path.join(".git").exists()
}

/// Remove a worktree and its branch
pub fn remove_worktree(
    project_path: &Path,
    session_name: &str,
    pane_label: &str,
    force: bool,
) -> Result<()> {
    let wt_path = pane_worktree_path(project_path, session_name, pane_label);
    let branch = pane_branch_name(session_name, pane_label);

    // Remove worktree
    let mut args = vec!["worktree", "remove"];
    if force {
        args.push("--force");
    }
    let wt_str = wt_path.to_string_lossy().to_string();
    args.push(&wt_str);

    let output = Command::new("git")
        .args(&args)
        .current_dir(project_path)
        .output()
        .context("Failed to run git worktree remove")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!("git worktree remove failed: {}", stderr.trim());
        // Fallback: remove directory manually
        if wt_path.exists() {
            std::fs::remove_dir_all(&wt_path).ok();
        }
    }

    // Remove branch
    let flag = if force { "-D" } else { "-d" };
    let _ = Command::new("git")
        .args(["branch", flag, &branch])
        .current_dir(project_path)
        .output();

    Ok(())
}

/// Merge a pane's branch into the current branch (should be main/master)
pub fn merge_branch(
    project_path: &Path,
    session_name: &str,
    pane_label: &str,
) -> Result<()> {
    let branch = pane_branch_name(session_name, pane_label);

    let output = Command::new("git")
        .args(["merge", &branch, "--no-ff", "-m", &format!("Merge legion session: {} ({})", session_name, pane_label)])
        .current_dir(project_path)
        .output()
        .context("Failed to run git merge")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("git merge failed (conflicts?): {}", stderr.trim());
    }

    Ok(())
}

/// Get the default branch name (main or master)
pub fn default_branch(project_path: &Path) -> String {
    let output = Command::new("git")
        .args(["symbolic-ref", "refs/remotes/origin/HEAD", "--short"])
        .current_dir(project_path)
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            let branch = String::from_utf8_lossy(&out.stdout).trim().to_string();
            // "origin/main" → "main"
            return branch.rsplit('/').next().unwrap_or("main").to_string();
        }
    }

    // Fallback: check if main exists, else master
    let check_main = Command::new("git")
        .args(["rev-parse", "--verify", "main"])
        .current_dir(project_path)
        .output();
    if check_main.map(|o| o.status.success()).unwrap_or(false) {
        "main".into()
    } else {
        "master".into()
    }
}

/// Create all worktrees for a session (leader + N workers)
pub fn create_session_worktrees(
    project_path: &Path,
    session_name: &str,
    worker_count: u16,
) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::with_capacity(1 + worker_count as usize);

    // Leader
    let leader_path = create_worktree(project_path, session_name, "Leader")?;
    paths.push(leader_path);

    // Workers
    for i in 1..=worker_count {
        let label = format!("Worker {}", i);
        let worker_path = create_worktree(project_path, session_name, &label)?;
        paths.push(worker_path);
    }

    Ok(paths)
}

/// Remove all worktrees for a session
pub fn remove_session_worktrees(
    project_path: &Path,
    session_name: &str,
    worker_count: u16,
    force: bool,
) -> Result<()> {
    remove_worktree(project_path, session_name, "Leader", force)?;
    for i in 1..=worker_count {
        let label = format!("Worker {}", i);
        remove_worktree(project_path, session_name, &label, force)?;
    }

    // Clean up session directory if empty
    let session_dir = legion_root(project_path).join(session_name);
    if session_dir.exists() {
        std::fs::remove_dir(&session_dir).ok(); // only succeeds if empty
    }

    Ok(())
}
```

**Step 2: Register the module**

In `crates/legion-tui/src/lib.rs`, add after line 6 (`pub mod ui;`):

```rust
pub mod worktree;
```

**Step 3: Compile and verify**

Run: `cargo build -p legion-tui`
Expected: Compiles cleanly.

**Step 4: Commit**

```bash
git add crates/legion-tui/src/worktree.rs crates/legion-tui/src/lib.rs
git commit -m "feat(tui): add worktree management module for session isolation"
```

---

### Task 4: App Session State and Lifecycle Methods

Add session tracking to `App` and methods for session create/resume/switch/complete.

**Files:**
- Modify: `crates/legion-tui/src/app.rs`

**Step 1: Add session fields to App**

In `crates/legion-tui/src/app.rs`, add imports at top (line 1-3):

```rust
use std::collections::HashMap;
use std::path::PathBuf;

use legion_core::orchestrate::{OrchestrateEngine, WorkerState};
use legion_db::{Provider, SquadSession};
```

Add new fields to `App` struct (after `show_dashboard: bool,` around line 103):

```rust
    // Session management
    pub current_session: Option<SquadSession>,
    pub project_path: Option<PathBuf>,
```

Initialize in `App::new()` (add before `saved_pane_configs`):

```rust
            current_session: None,
            project_path: None,
```

**Step 2: Add session lifecycle methods**

Add these methods to `impl App` (before the `// --- Menu navigation ---` comment):

```rust
    /// Get the current session name for display
    pub fn session_name(&self) -> &str {
        self.current_session
            .as_ref()
            .map(|s| s.name.as_str())
            .unwrap_or("(no session)")
    }

    /// Create a new session: create worktrees, save to DB
    pub fn create_session(&mut self, name: &str, worker_count: u16) -> anyhow::Result<Vec<PathBuf>> {
        let project_path = self.project_path.as_ref()
            .ok_or_else(|| anyhow::anyhow!("No project path set"))?;

        let paths = crate::worktree::create_session_worktrees(project_path, name, worker_count)?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let session = SquadSession {
            name: name.to_string(),
            project_path: project_path.to_string_lossy().to_string(),
            worker_count: worker_count as i64,
            status: "active".to_string(),
            created_at: now,
            completed_at: None,
        };

        if let Ok(repo) = legion_db::open_db() {
            repo.upsert_squad_session(&session)?;
        }

        self.current_session = Some(session);
        Ok(paths)
    }

    /// Get worktree path for a pane in the current session
    pub fn pane_worktree(&self, pane_label: &str) -> Option<PathBuf> {
        let project_path = self.project_path.as_ref()?;
        let session = self.current_session.as_ref()?;
        Some(crate::worktree::pane_worktree_path(project_path, &session.name, pane_label))
    }
```

**Step 3: Compile and verify**

Run: `cargo build -p legion-tui`
Expected: Compiles cleanly.

**Step 4: Commit**

```bash
git add crates/legion-tui/src/app.rs
git commit -m "feat(app): add session state and lifecycle methods"
```

---

### Task 5: Restructure Ctrl+P Menu

Change Ctrl+P to open a Main Menu with: Switch Models, Switch Session, Complete Session, Quit.

**Files:**
- Modify: `crates/legion-tui/src/app.rs` (MainMenuItem enum, enter_submenu, toggle_popup)
- Modify: `crates/legion-tui/src/input.rs` (handle_popup_mode, new Session/Complete handlers)
- Modify: `crates/legion-tui/src/ui.rs` (draw_popup, draw_main_menu)

**Step 1: Update PopupMenu and MainMenuItem enums in app.rs**

Replace the `PopupMenu` enum (lines 21-26):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupMenu {
    Main,
    Provider,
    Model,
    Matrix,
    SessionList,
    CompleteSession,
}
```

Replace the `MainMenuItem` enum and impl (lines 44-57):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainMenuItem {
    SwitchModels,
    SwitchSession,
    CompleteSession,
    Quit,
}

impl MainMenuItem {
    pub fn label(&self) -> &'static str {
        match self {
            Self::SwitchModels => "Switch Models",
            Self::SwitchSession => "Switch Session",
            Self::CompleteSession => "Complete Session",
            Self::Quit => "Quit",
        }
    }
}
```

**Step 2: Add session list state to App**

Add fields to `App` struct (after `project_path`):

```rust
    pub session_list: Vec<SquadSession>,
    pub session_list_index: usize,
    pub complete_merge_index: usize,  // 0=Merge, 1=Keep, 2=Discard
```

Initialize in `App::new()`:

```rust
            session_list: Vec::new(),
            session_list_index: 0,
            complete_merge_index: 0,
```

**Step 3: Update main_menu_items and toggle_popup**

Replace `main_menu_items()` (line 386-388):

```rust
    pub fn main_menu_items() -> &'static [MainMenuItem] {
        &[MainMenuItem::SwitchModels, MainMenuItem::SwitchSession, MainMenuItem::CompleteSession, MainMenuItem::Quit]
    }
```

Replace `toggle_popup()` to open Main Menu instead of Matrix:

```rust
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
```

**Step 4: Update enter_submenu**

Replace `enter_submenu()` (lines 403-419):

```rust
    pub fn enter_submenu(&mut self) {
        if let AppMode::Popup(PopupMenu::Main) = self.mode {
            let items = Self::main_menu_items();
            if self.menu_index < items.len() {
                match items[self.menu_index] {
                    MainMenuItem::SwitchModels => {
                        self.mode = AppMode::Popup(PopupMenu::Matrix);
                        self.matrix_row = 0;
                        self.matrix_col = MatrixCol::Provider;
                    }
                    MainMenuItem::SwitchSession => {
                        self.load_session_list();
                        self.mode = AppMode::Popup(PopupMenu::SessionList);
                        self.session_list_index = 0;
                    }
                    MainMenuItem::CompleteSession => {
                        self.mode = AppMode::Popup(PopupMenu::CompleteSession);
                        self.complete_merge_index = 0;
                    }
                    MainMenuItem::Quit => {
                        self.should_quit = true;
                    }
                }
            }
        }
    }
```

**Step 5: Add load_session_list method**

Add to `impl App`:

```rust
    /// Load squad sessions from DB for the session list popup
    pub fn load_session_list(&mut self) {
        if let Ok(repo) = legion_db::open_db() {
            self.session_list = repo.list_squad_sessions().unwrap_or_default();
        }
    }
```

**Step 6: Update menu_up/menu_down for new popup types**

In `menu_up()` (around line 498), update the `len` match to handle SessionList and CompleteSession:

```rust
    pub fn menu_up(&mut self) {
        let len = match self.mode {
            AppMode::Popup(PopupMenu::Main) => Self::main_menu_items().len(),
            AppMode::Popup(PopupMenu::Provider) => self.providers.len(),
            AppMode::Popup(PopupMenu::Model) => {
                self.target_provider_models().map(|m| m.len()).unwrap_or(0)
            }
            AppMode::Popup(PopupMenu::Matrix) => self.matrix_row_count(),
            AppMode::Popup(PopupMenu::SessionList) => self.session_list.len() + 1, // +1 for "New Session"
            AppMode::Popup(PopupMenu::CompleteSession) => 3, // Merge, Keep, Discard
            _ => return,
        };
        let idx = match self.mode {
            AppMode::Popup(PopupMenu::Main) => &mut self.menu_index,
            AppMode::Popup(PopupMenu::Matrix) => &mut self.matrix_row,
            AppMode::Popup(PopupMenu::SessionList) => &mut self.session_list_index,
            AppMode::Popup(PopupMenu::CompleteSession) => &mut self.complete_merge_index,
            _ => &mut self.submenu_index,
        };
        *idx = if *idx > 0 { *idx - 1 } else { len.saturating_sub(1) };
    }
```

Apply the same changes to `menu_down()`.

**Step 7: Update input.rs popup handler**

In `crates/legion-tui/src/input.rs`, update `handle_popup_mode()` to route SessionList and CompleteSession:

```rust
fn handle_popup_mode(app: &mut App, key: KeyEvent) -> InputResult {
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
        AppMode::Popup(PopupMenu::SessionList) => handle_session_list_keys(app, key),
        AppMode::Popup(PopupMenu::CompleteSession) => handle_complete_session_keys(app, key),
        _ => {}
    }

    InputResult::Continue
}
```

Add new key handlers:

```rust
fn handle_session_list_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.mode = AppMode::Popup(PopupMenu::Main);
        }
        KeyCode::Up | KeyCode::Char('k') => app.menu_up(),
        KeyCode::Down | KeyCode::Char('j') => app.menu_down(),
        KeyCode::Enter => {
            // Last item = "New Session", others = existing sessions
            if app.session_list_index >= app.session_list.len() {
                // TODO: Task 8 will handle creating new session
                tracing::info!("New session requested");
            } else {
                // TODO: Task 8 will handle switching session
                let session = &app.session_list[app.session_list_index];
                tracing::info!("Switch to session: {}", session.name);
            }
            app.mode = AppMode::Normal;
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
            // TODO: Task 8 will implement actual merge/keep/discard
            let action = match app.complete_merge_index {
                0 => "merge",
                1 => "keep",
                _ => "discard",
            };
            tracing::info!("Complete session with action: {}", action);
            app.mode = AppMode::Normal;
        }
        _ => {}
    }
}
```

**Step 8: Update ui.rs to render new popup types**

In `crates/legion-tui/src/ui.rs`, update `draw_popup()` (lines 251-261):

```rust
fn draw_popup(frame: &mut Frame, app: &App, menu: PopupMenu) {
    let area = centered_rect(60, 60, frame.area());
    frame.render_widget(Clear, area);

    match menu {
        PopupMenu::Main => draw_main_menu(frame, app, area),
        PopupMenu::Matrix => draw_matrix(frame, app, area),
        PopupMenu::Provider => draw_provider_menu(frame, app, area),
        PopupMenu::Model => draw_model_menu(frame, app, area),
        PopupMenu::SessionList => draw_session_list(frame, app, area),
        PopupMenu::CompleteSession => draw_complete_session(frame, app, area),
    }
}
```

Update `draw_main_menu()` — replace the `value` match (lines 276-283) to show context for new items:

```rust
let value = match item {
    MainMenuItem::SwitchModels => {
        let n = app.panes.len();
        if n == 0 { "[no panes]".to_string() }
        else { format!("[{} pane{}]", n, if n == 1 { "" } else { "s" }) }
    }
    MainMenuItem::SwitchSession => {
        format!("[{}]", app.session_name())
    }
    MainMenuItem::CompleteSession => String::new(),
    MainMenuItem::Quit => String::new(),
};
```

Also update the separator position — currently hardcoded to before index 1. Change to before the last item (Quit):

```rust
    let quit_idx = items.len() - 1;
    let mut final_items = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        if i == quit_idx {
            final_items.push(ListItem::new(Line::from(Span::styled(
                "  ─".repeat(12),
                Style::default().fg(Color::DarkGray),
            ))));
        }
        final_items.push(item);
    }
```

Add the two new draw functions:

```rust
fn draw_session_list(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" Sessions [ESC] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::DarkGray));

    let mut items: Vec<ListItem> = Vec::new();

    let current_name = app.current_session.as_ref().map(|s| s.name.as_str());

    for (i, session) in app.session_list.iter().enumerate() {
        let selected = i == app.session_list_index;
        let prefix = if selected { "> " } else { "  " };

        let icon = if current_name == Some(&session.name) {
            "● "
        } else if session.status == "completed" {
            "✓ "
        } else {
            "○ "
        };

        let style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else if session.status == "completed" {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default().fg(Color::White)
        };

        let pane_count = 1 + session.worker_count;
        items.push(ListItem::new(Line::from(vec![
            Span::raw(prefix),
            Span::styled(icon, style),
            Span::styled(&session.name, style),
            Span::styled(
                format!("  {} panes", pane_count),
                Style::default().fg(Color::DarkGray),
            ),
        ])));
    }

    // "New Session" item
    let new_selected = app.session_list_index >= app.session_list.len();
    let new_prefix = if new_selected { "> " } else { "  " };
    let new_style = if new_selected {
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Green)
    };
    items.push(ListItem::new(Line::from(vec![
        Span::raw(new_prefix),
        Span::styled("[+] New Session", new_style),
    ])));

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

    let items: Vec<ListItem> = options
        .iter()
        .zip(descriptions.iter())
        .enumerate()
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
                Line::from(Span::styled(
                    format!("    {}", desc),
                    Style::default().fg(Color::DarkGray),
                )),
            ])
        })
        .collect();

    frame.render_widget(List::new(items).block(block), area);
}
```

**Step 9: Update the matrix back navigation**

In `handle_matrix_keys` (input.rs), change Esc to go back to Main Menu instead of closing popup:

```rust
fn handle_matrix_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.mode = AppMode::Popup(PopupMenu::Main);
        }
        // ... rest unchanged
    }
}
```

**Step 10: Compile and test**

Run: `cargo build`
Expected: Compiles cleanly. Ctrl+P now opens Main Menu.

**Step 11: Commit**

```bash
git add crates/legion-tui/
git commit -m "feat(menu): restructure Ctrl+P with Switch Models/Session/Complete/Quit"
```

---

### Task 6: Session Header Display

Show the current session name in the TUI header bar.

**Files:**
- Modify: `crates/legion-tui/src/ui.rs:44-78` (draw_header)

**Step 1: Update draw_header to show session name**

In `draw_header()`, add session info after the provider/model display. Replace the `header` Line construction (lines 61-74):

```rust
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

    spans.extend([
        Span::raw("  "),
        Span::styled("[", Style::default().fg(Color::DarkGray)),
        Span::styled(provider_name, Style::default().fg(Color::Yellow)),
        Span::styled(" → ", Style::default().fg(Color::DarkGray)),
        Span::styled(model_name, Style::default().fg(Color::Magenta)),
        Span::styled("]", Style::default().fg(Color::DarkGray)),
        indicator,
    ]);

    let header = Line::from(spans);
```

**Step 2: Compile and verify**

Run: `cargo build -p legion-tui`
Expected: Compiles.

**Step 3: Commit**

```bash
git add crates/legion-tui/src/ui.rs
git commit -m "feat(ui): show session name in header bar"
```

---

### Task 7: Startup Session Selection Flow

Modify `run_squad()` and `cmd_squad()` to:
1. Detect the current project path (git root)
2. Query existing sessions from DB
3. If sessions exist, present selection before entering TUI
4. Create/resume session with worktrees

**Files:**
- Modify: `crates/legion-tui/src/lib.rs:57-136` (run_squad)
- Modify: `crates/legion-cli/src/main.rs:341-438` (cmd_squad)

**Step 1: Add project_path detection helper**

In `crates/legion-tui/src/lib.rs`, add a helper function (before `run_event_loop`):

```rust
/// Detect the current project root via `git rev-parse --show-toplevel`
fn detect_project_path() -> Option<std::path::PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Some(std::path::PathBuf::from(path))
    } else {
        None
    }
}
```

**Step 2: Add pre-TUI session selection**

Add a function that runs before the TUI to select/create a session. This uses simple stdin/stdout since the TUI hasn't started yet:

```rust
/// Pre-TUI session selection (runs before alternate screen)
fn select_session_interactive(project_path: &std::path::Path, default_workers: u16) -> Result<(String, u16, bool)> {
    // Returns (session_name, worker_count, is_resume)
    let sessions: Vec<legion_db::SquadSession> = legion_db::open_db()
        .and_then(|repo| repo.list_active_squad_sessions())
        .unwrap_or_default();

    if sessions.is_empty() {
        // No sessions — prompt for name
        eprint!("Session name: ");
        let mut name = String::new();
        std::io::stdin().read_line(&mut name)?;
        let name = name.trim().to_string();
        if name.is_empty() {
            anyhow::bail!("Session name cannot be empty");
        }
        return Ok((name, default_workers, false));
    }

    // Show session list
    eprintln!("\n  Active sessions:");
    for (i, s) in sessions.iter().enumerate() {
        let panes = 1 + s.worker_count;
        eprintln!("  {}. {} ({} panes)", i + 1, s.name, panes);
    }
    eprintln!("  {}. [New Session]", sessions.len() + 1);
    eprint!("\n  Select [1-{}]: ", sessions.len() + 1);

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let choice: usize = input.trim().parse().unwrap_or(0);

    if choice >= 1 && choice <= sessions.len() {
        let session = &sessions[choice - 1];
        Ok((session.name.clone(), session.worker_count as u16, true))
    } else {
        // New session
        eprint!("  Session name: ");
        let mut name = String::new();
        std::io::stdin().read_line(&mut name)?;
        let name = name.trim().to_string();
        if name.is_empty() {
            anyhow::bail!("Session name cannot be empty");
        }
        Ok((name, default_workers, false))
    }
}
```

**Step 3: Update run_squad to accept session info**

Change `run_squad` signature and use session info:

```rust
pub async fn run_squad(worker_count: u16, base_port: u16) -> Result<()> {
    // Detect project path
    let project_path = detect_project_path()
        .ok_or_else(|| anyhow::anyhow!("Not in a git repository"))?;

    // Session selection (before TUI starts)
    let (session_name, actual_workers, is_resume) =
        select_session_interactive(&project_path, worker_count)?;

    // Setup terminal with mouse support
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and load providers
    let mut app = App::new();
    app.load_from_db();
    app.project_path = Some(project_path.clone());

    // Create or verify worktrees
    let worktree_paths = if is_resume {
        // Verify worktrees exist for resume
        let mut paths = Vec::new();
        for i in 0..=actual_workers {
            let label = if i == 0 { "Leader".to_string() } else { format!("Worker {}", i) };
            let wt = worktree::pane_worktree_path(&project_path, &session_name, &label);
            if !worktree::worktree_exists(&wt) {
                // Worktree missing, recreate it
                tracing::warn!("Worktree missing for {}, recreating", label);
                let _ = worktree::create_worktree(&project_path, &session_name, &label);
            }
            paths.push(wt);
        }
        // Load session from DB
        if let Ok(repo) = legion_db::open_db() {
            app.current_session = repo.get_squad_session(&session_name).ok().flatten();
        }
        paths
    } else {
        // Create new session
        let paths = app.create_session(&session_name, actual_workers)?;
        paths
    };

    // Cache terminal size
    let size = terminal.size()?;
    app.term_size = (size.width, size.height);

    // Calculate PTY sizes
    let content_height = size.height.saturating_sub(2);
    let leader_width = (size.width as u32 * app.leader_ratio as u32 / 100) as u16;
    let leader_pty_rows = content_height.saturating_sub(2);
    let leader_pty_cols = leader_width.saturating_sub(2);
    let worker_width = size.width.saturating_sub(leader_width).saturating_sub(1);
    let worker_height = content_height / actual_workers;
    let worker_pty_rows = worker_height.saturating_sub(2);
    let worker_pty_cols = worker_width.saturating_sub(2);

    // System prompts
    let leader_prompt = claudemd::leader_instructions(actual_workers);
    let worker_prompts: Vec<String> = (1..=actual_workers)
        .map(|id| claudemd::worker_instructions(id))
        .collect();

    // Port assignments
    let orchestrate_port = base_port + 2000;
    let leader_proxy = base_port;
    let leader_control = base_port + 1000;

    // Spawn panes with worktree paths
    app.add_pane(
        leader_pty_rows, leader_pty_cols, leader_proxy, leader_control,
        "Leader".into(), false, None, Some(orchestrate_port), Some(&leader_prompt),
        Some(worktree_paths[0].as_path()), is_resume,
    );

    for i in 0..actual_workers {
        let proxy = base_port + i + 1;
        let control = base_port + 1000 + i + 1;
        let label = format!("Worker {}", i + 1);
        app.add_pane(
            worker_pty_rows, worker_pty_cols, proxy, control,
            label, true, Some(i + 1), Some(orchestrate_port),
            Some(&worker_prompts[i as usize]),
            Some(worktree_paths[1 + i as usize].as_path()), is_resume,
        );
    }

    // Start orchestration
    let engine = legion_core::OrchestrateEngine::new(actual_workers);
    app.orchestrate = Some(engine.clone());
    let orch_api = legion_core::OrchestrateApi::new(engine, orchestrate_port);
    let (orch_tx, orch_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        if let Err(e) = orch_api.start_with_signal(Some(orch_tx)).await {
            tracing::error!("Orchestrate API error: {}", e);
        }
    });
    orch_rx.await.ok();

    // Main event loop
    let result = run_event_loop(&mut terminal, &mut app).await;

    // Cleanup
    app.kill_all();
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture);
    let _ = terminal.show_cursor();

    std::process::exit(if result.is_ok() { 0 } else { 1 });
}
```

**Step 4: Compile and verify**

Run: `cargo build`
Expected: Compiles. `legion squad` now prompts for session selection before starting.

**Step 5: Commit**

```bash
git add crates/legion-tui/src/lib.rs
git commit -m "feat(squad): session selection at startup with worktree creation/resume"
```

---

### Task 8: Wire Up Session Actions (Switch + Complete)

Implement the actual logic behind session list Enter (switch) and complete session Enter (merge/keep/discard).

**Files:**
- Modify: `crates/legion-tui/src/input.rs` (handle_session_list_keys, handle_complete_session_keys)
- Modify: `crates/legion-tui/src/app.rs` (add switch/complete methods)

**Step 1: Add complete_session method to App**

In `crates/legion-tui/src/app.rs`, add:

```rust
    /// Mark current session as completed with given merge strategy
    /// Returns Ok(true) if session was completed, Ok(false) if no session active
    pub fn complete_current_session(&mut self, strategy: &str) -> anyhow::Result<bool> {
        let session = match self.current_session.take() {
            Some(s) => s,
            None => return Ok(false),
        };
        let project_path = match self.project_path.as_ref() {
            Some(p) => p.clone(),
            None => return Ok(false),
        };

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        match strategy {
            "merge" => {
                // Checkout default branch, merge each pane's branch
                let default_branch = crate::worktree::default_branch(&project_path);
                let _ = std::process::Command::new("git")
                    .args(["checkout", &default_branch])
                    .current_dir(&project_path)
                    .output();

                let pane_labels = std::iter::once("Leader".to_string())
                    .chain((1..=session.worker_count).map(|i| format!("Worker {}", i)));

                for label in pane_labels {
                    if let Err(e) = crate::worktree::merge_branch(&project_path, &session.name, &label) {
                        tracing::error!("Merge failed for {}: {}", label, e);
                        // Put session back — merge conflict
                        self.current_session = Some(session);
                        return Err(e);
                    }
                }

                // Clean up worktrees after successful merge
                crate::worktree::remove_session_worktrees(
                    &project_path, &session.name, session.worker_count as u16, false,
                )?;
            }
            "discard" => {
                crate::worktree::remove_session_worktrees(
                    &project_path, &session.name, session.worker_count as u16, true,
                )?;
            }
            _ => {
                // "keep" — do nothing to worktrees
            }
        }

        // Update DB
        if let Ok(repo) = legion_db::open_db() {
            repo.complete_squad_session(&session.name, now)?;
        }

        Ok(true)
    }
```

**Step 2: Wire up complete_session_keys in input.rs**

Replace the TODO in `handle_complete_session_keys`:

```rust
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

            // Kill all PTYs first
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
            app.should_quit = true; // Exit TUI after completing session
        }
        _ => {}
    }
}
```

**Step 3: Compile and verify**

Run: `cargo build`
Expected: Compiles.

**Step 4: Commit**

```bash
git add crates/legion-tui/src/input.rs crates/legion-tui/src/app.rs
git commit -m "feat(session): wire up complete session with merge/keep/discard strategies"
```

---

### Task 9: Full Build + Integration Test

Verify everything compiles and the session flow works end-to-end.

**Files:** (no changes — verification only)

**Step 1: Build the entire workspace**

Run: `cargo build`
Expected: No errors, no warnings (except possibly unused warnings for not-yet-called code).

**Step 2: Run all tests**

Run: `cargo test`
Expected: All tests pass (including new `squad_session_crud` and `squad_session_list_active_only`).

**Step 3: Manual smoke test**

1. `cargo run -- import` — seed providers
2. `cd /tmp && mkdir test-repo && cd test-repo && git init && echo "test" > README.md && git add . && git commit -m "init"`
3. `cd /tmp/test-repo && cargo run --manifest-path /path/to/legion/Cargo.toml -- squad --workers 1`
4. Should see session prompt: "Session name: " — type "test-session"
5. Worktree created at `/tmp/test-repo-legion/test-session/leader/` etc.
6. TUI shows session name in header
7. Ctrl+P → Main Menu shows: Switch Models, Switch Session, Complete Session, Quit
8. Ctrl+Q to exit
9. Re-run same command → should list "test-session" for resume

**Step 4: Commit if any fixes needed**

```bash
git add -A
git commit -m "fix: address integration test issues"
```

---

## Summary of Changes by File

| File | Changes |
|------|---------|
| `crates/legion-db/src/schema.rs` | Add `squad_sessions` table |
| `crates/legion-db/src/repo.rs` | Add `SquadSession` struct + 6 CRUD methods + 2 tests |
| `crates/legion-db/src/lib.rs` | Export `SquadSession` |
| `crates/legion-tui/src/pty.rs` | Add `working_dir` + `continue_session` params |
| `crates/legion-tui/src/worktree.rs` | **NEW** — git worktree create/remove/merge/verify |
| `crates/legion-tui/src/app.rs` | Session state, lifecycle methods, menu restructure |
| `crates/legion-tui/src/input.rs` | Session list + complete session key handlers |
| `crates/legion-tui/src/ui.rs` | Session list popup, complete dialog, header session name |
| `crates/legion-tui/src/lib.rs` | Session selection flow, worktree module, detect_project_path |
