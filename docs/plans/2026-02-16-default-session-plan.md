# Default Session & Session Lifecycle Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a locked default session that auto-resumes on startup, with full session delete/complete lifecycle and record migration.

**Architecture:** DB schema adds `is_default` to squad_sessions and `origin_session` to tickets. Default session Leader uses main repo (no worktree). Session list popup gains d/c/n/x keys. Complete session merges code then lets user delete or migrate records before removing the session entry.

**Tech Stack:** Rust, rusqlite, ratatui, legion-db, legion-tui

---

### Task 1: DB Schema — Add `is_default` and `origin_session` columns

**Files:**
- Modify: `crates/legion-db/src/schema.rs`
- Modify: `crates/legion-db/src/repo.rs`

**Step 1: Update schema.rs**

In `crates/legion-db/src/schema.rs`, add `is_default` column to `squad_sessions` table and `origin_session` to `tickets` table.

```rust
// In squad_sessions CREATE TABLE, after completed_at:
//     is_default INTEGER NOT NULL DEFAULT 0

// In tickets CREATE TABLE, after updated_at:
//     origin_session TEXT
```

Full replacement for `squad_sessions`:
```sql
CREATE TABLE IF NOT EXISTS squad_sessions (
    name TEXT PRIMARY KEY,
    project_path TEXT NOT NULL,
    worker_count INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    created_at INTEGER NOT NULL,
    completed_at INTEGER,
    is_default INTEGER NOT NULL DEFAULT 0
);
```

Full replacement for `tickets`:
```sql
CREATE TABLE IF NOT EXISTS tickets (
    id INTEGER PRIMARY KEY,
    session_name TEXT NOT NULL,
    title TEXT NOT NULL,
    prompt TEXT NOT NULL,
    context TEXT,
    criteria TEXT,
    status TEXT NOT NULL DEFAULT 'queued',
    assigned_worker INTEGER,
    team_mode TEXT NOT NULL DEFAULT 'tech_lead_team',
    iteration INTEGER NOT NULL DEFAULT 0,
    max_iterations INTEGER NOT NULL DEFAULT 5,
    feedback TEXT,
    summary TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    origin_session TEXT
);
```

**Step 2: Update SquadSession struct in repo.rs**

Add `is_default` field:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SquadSession {
    pub name: String,
    pub project_path: String,
    pub worker_count: i64,
    pub status: String,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub is_default: bool,
}
```

Add `origin_session` to TicketRow:
```rust
pub struct TicketRow {
    // ... existing fields ...
    pub origin_session: Option<String>,
}
```

**Step 3: Update all SquadSession SQL queries in repo.rs**

Every query that reads/writes `squad_sessions` needs the `is_default` column. Update:
- `upsert_squad_session` — add `is_default` to INSERT
- `get_squad_session` — add `is_default` to SELECT
- `list_squad_sessions` — add `is_default` to SELECT
- `list_active_squad_sessions` — add `is_default` to SELECT

For upsert:
```rust
pub fn upsert_squad_session(&self, session: &SquadSession) -> Result<()> {
    self.conn.execute(
        "INSERT OR REPLACE INTO squad_sessions (name, project_path, worker_count, status, created_at, completed_at, is_default) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            session.name,
            session.project_path,
            session.worker_count,
            session.status,
            session.created_at,
            session.completed_at,
            session.is_default as i32,
        ],
    )?;
    Ok(())
}
```

For all SELECT queries, change the column list to include `is_default` and parse it:
```rust
// In the row mapping closure:
is_default: row.get::<_, i32>(6)? != 0,
// Note: completed_at is index 5, is_default is index 6
```

Similarly update all TicketRow queries to include `origin_session` column.

For `insert_ticket`:
```rust
// Add origin_session as 16th param
"INSERT OR REPLACE INTO tickets (id, session_name, title, prompt, context, criteria, status, assigned_worker, team_mode, iteration, max_iterations, feedback, summary, created_at, updated_at, origin_session) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
// Add: ticket.origin_session,
```

For `list_tickets_by_session` SELECT, add `origin_session` to the column list and parse: `origin_session: row.get(15)?`

**Step 4: Add new DB methods**

```rust
/// Get default session for a project path
pub fn get_default_squad_session(&self, project_path: &str) -> Result<Option<SquadSession>> {
    let mut stmt = self.conn.prepare(
        "SELECT name, project_path, worker_count, status, created_at, completed_at, is_default FROM squad_sessions WHERE is_default = 1 AND project_path = ? LIMIT 1"
    )?;
    let mut rows = stmt.query(params![project_path])?;
    if let Some(row) = rows.next()? {
        Ok(Some(SquadSession {
            name: row.get(0)?,
            project_path: row.get(1)?,
            worker_count: row.get(2)?,
            status: row.get(3)?,
            created_at: row.get(4)?,
            completed_at: row.get(5)?,
            is_default: row.get::<_, i32>(6)? != 0,
        }))
    } else {
        Ok(None)
    }
}

/// Delete all tickets and ticket_logs for a session
pub fn delete_session_tickets(&self, session_name: &str) -> Result<()> {
    self.conn.execute(
        "DELETE FROM ticket_logs WHERE session_name = ?",
        params![session_name],
    )?;
    self.conn.execute(
        "DELETE FROM tickets WHERE session_name = ?",
        params![session_name],
    )?;
    Ok(())
}

/// Migrate tickets and logs from one session to another, recording origin
pub fn migrate_tickets_to_session(&self, from_session: &str, to_session: &str) -> Result<()> {
    self.conn.execute(
        "UPDATE tickets SET session_name = ?1, origin_session = ?2 WHERE session_name = ?2",
        params![to_session, from_session],
    )?;
    self.conn.execute(
        "UPDATE ticket_logs SET session_name = ?1 WHERE session_name = ?2",
        params![to_session, from_session],
    )?;
    Ok(())
}

/// Count non-completed tickets for a session (queued + in_progress)
pub fn count_pending_tickets(&self, session_name: &str) -> Result<usize> {
    let count: i64 = self.conn.query_row(
        "SELECT COUNT(*) FROM tickets WHERE session_name = ? AND status IN ('queued', 'in_progress')",
        params![session_name],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}

/// Count all tickets for a session
pub fn count_tickets(&self, session_name: &str) -> Result<usize> {
    let count: i64 = self.conn.query_row(
        "SELECT COUNT(*) FROM tickets WHERE session_name = ?",
        params![session_name],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}

/// Count ticket log entries for a session
pub fn count_ticket_logs(&self, session_name: &str) -> Result<usize> {
    let count: i64 = self.conn.query_row(
        "SELECT COUNT(*) FROM ticket_logs WHERE session_name = ?",
        params![session_name],
        |row| row.get(0),
    )?;
    Ok(count as usize)
}
```

**Step 5: Update existing tests and add new tests**

Update `squad_session_crud` and `squad_session_list_active_only` tests to include `is_default: false` in all SquadSession construction.

Add new test:
```rust
#[test]
fn default_squad_session() {
    let repo = test_repo();

    // No default initially
    assert!(repo.get_default_squad_session("/tmp/proj").unwrap().is_none());

    // Insert default
    repo.upsert_squad_session(&SquadSession {
        name: "main".into(),
        project_path: "/tmp/proj".into(),
        worker_count: 3,
        status: "active".into(),
        created_at: 1000,
        completed_at: None,
        is_default: true,
    }).unwrap();

    let def = repo.get_default_squad_session("/tmp/proj").unwrap().unwrap();
    assert_eq!(def.name, "main");
    assert!(def.is_default);

    // Different project path returns None
    assert!(repo.get_default_squad_session("/tmp/other").unwrap().is_none());
}

#[test]
fn delete_session_tickets_and_migrate() {
    let repo = test_repo();

    // Insert tickets for session "feat"
    repo.insert_ticket(&TicketRow {
        id: 1,
        session_name: "feat".into(),
        title: "task 1".into(),
        prompt: "do stuff".into(),
        context: None,
        criteria: None,
        status: "completed".into(),
        assigned_worker: None,
        team_mode: "tech_lead_team".into(),
        iteration: 1,
        max_iterations: 5,
        feedback: None,
        summary: None,
        created_at: 100,
        updated_at: 100,
        origin_session: None,
    }).unwrap();
    repo.append_ticket_log(1, "feat", "log entry", 100).unwrap();

    // Delete all
    repo.delete_session_tickets("feat").unwrap();
    assert!(repo.list_tickets_by_session("feat").unwrap().is_empty());
    assert!(repo.get_ticket_logs(1, "feat").unwrap().is_empty());
}

#[test]
fn migrate_tickets_between_sessions() {
    let repo = test_repo();

    repo.insert_ticket(&TicketRow {
        id: 1,
        session_name: "feat".into(),
        title: "task 1".into(),
        prompt: "do stuff".into(),
        context: None, criteria: None,
        status: "completed".into(),
        assigned_worker: None,
        team_mode: "tech_lead_team".into(),
        iteration: 1, max_iterations: 5,
        feedback: None, summary: None,
        created_at: 100, updated_at: 100,
        origin_session: None,
    }).unwrap();
    repo.append_ticket_log(1, "feat", "log entry", 100).unwrap();

    // Migrate feat -> main
    repo.migrate_tickets_to_session("feat", "main").unwrap();

    // Now ticket is under "main" with origin_session = "feat"
    let tickets = repo.list_tickets_by_session("main").unwrap();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].origin_session.as_deref(), Some("feat"));

    // Logs also migrated
    let logs = repo.get_ticket_logs(1, "main").unwrap();
    assert_eq!(logs.len(), 1);

    // Old session has nothing
    assert!(repo.list_tickets_by_session("feat").unwrap().is_empty());
}
```

**Step 6: Run tests**

Run: `cargo test -p legion-db`
Expected: All tests pass

**Step 7: Commit**

```bash
git add crates/legion-db/src/schema.rs crates/legion-db/src/repo.rs
git commit -m "feat(db): add is_default to squad_sessions, origin_session to tickets, session lifecycle methods"
```

---

### Task 2: Worktree — Support default session (skip Leader worktree)

**Files:**
- Modify: `crates/legion-tui/src/worktree.rs`

**Step 1: Add `create_session_worktrees_default` variant**

Add a new function that creates only worker worktrees (Leader uses main repo):

```rust
/// Create worktrees for a default session (Leader uses main repo, workers get worktrees)
pub fn create_default_session_worktrees(
    project_path: &Path,
    session_name: &str,
    worker_count: u16,
) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::with_capacity(1 + worker_count as usize);
    // Leader path = project_path itself (no worktree)
    paths.push(project_path.to_path_buf());
    for i in 1..=worker_count {
        paths.push(create_worktree(
            project_path,
            session_name,
            &format!("Worker {}", i),
        )?);
    }
    Ok(paths)
}
```

**Step 2: Add `remove_default_session_worktrees`**

Only removes worker worktrees (Leader = main repo, not touched):

```rust
/// Remove worktrees for a default session (only workers, not Leader)
pub fn remove_default_session_worktrees(
    project_path: &Path,
    session_name: &str,
    worker_count: u16,
    force: bool,
) -> Result<()> {
    // Skip Leader — it's the main repo
    for i in 1..=worker_count {
        remove_worktree(project_path, session_name, &format!("Worker {}", i), force)?;
    }

    let session_dir = legion_root(project_path).join(session_name);
    if session_dir.exists() {
        std::fs::remove_dir(&session_dir).ok();
    }

    Ok(())
}
```

**Step 3: Commit**

```bash
git add crates/legion-tui/src/worktree.rs
git commit -m "feat(worktree): add default session worktree helpers (skip Leader)"
```

---

### Task 3: App — Default session creation and updated lifecycle

**Files:**
- Modify: `crates/legion-tui/src/app.rs`

**Step 1: Add new PopupMenu variants and App state fields**

Add to `PopupMenu` enum:
```rust
SessionDeleteConfirm,   // Confirm delete session from session list
CompleteRecordChoice,    // Choose delete or migrate records after complete
```

Add to `App` struct fields (near existing session fields):
```rust
pub session_delete_target: Option<String>,       // session name to delete
pub session_delete_pending_count: usize,         // pending ticket count for confirm display
pub session_delete_ticket_count: usize,          // total ticket count
pub session_delete_log_count: usize,             // total log count
pub complete_record_choice: usize,               // 0=delete records, 1=migrate to default
pub complete_session_name: Option<String>,        // session being completed
```

Initialize them in `App::new()` with default values.

**Step 2: Update `create_session` to accept `is_default`**

```rust
pub fn create_session(&mut self, name: &str, worker_count: u16, is_default: bool) -> anyhow::Result<Vec<PathBuf>> {
    let project_path = self.project_path.as_ref()
        .ok_or_else(|| anyhow::anyhow!("No project path set"))?;

    let paths = if is_default {
        crate::worktree::create_default_session_worktrees(project_path, name, worker_count)?
    } else {
        crate::worktree::create_session_worktrees(project_path, name, worker_count)?
    };

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
        is_default,
    };

    if let Ok(repo) = legion_db::open_db() {
        repo.upsert_squad_session(&session)?;
    }

    self.current_session = Some(session);
    Ok(paths)
}
```

**Step 3: Update `start_session` to handle default session Leader path**

In `start_session`, when `is_resume` is true and session is default, Leader path = project_path:

```rust
// In the is_resume branch, replace the worktree path logic:
let worktree_paths = if is_resume {
    // Check if this is a default session
    let is_default_session = if let Ok(repo) = legion_db::open_db() {
        let sess = repo.get_squad_session(name).ok().flatten();
        self.current_session = sess.clone();
        sess.map(|s| s.is_default).unwrap_or(false)
    } else {
        false
    };

    let mut paths = Vec::new();
    for i in 0..=worker_count {
        let label = if i == 0 { "Leader".to_string() } else { format!("Worker {}", i) };
        if i == 0 && is_default_session {
            // Default session Leader = main repo
            paths.push(project_path.clone());
        } else {
            let wt = crate::worktree::pane_worktree_path(&project_path, name, &label);
            if !crate::worktree::worktree_exists(&wt) {
                let _ = crate::worktree::create_worktree(&project_path, name, &label);
            }
            paths.push(wt);
        }
    }
    paths
} else {
    // Determine if creating default session
    let is_default = /* passed as parameter or checked */;
    self.create_session(name, worker_count, is_default)?
};
```

To pass `is_default` cleanly, change `start_session` signature:
```rust
pub fn start_session(&mut self, name: &str, worker_count: u16, is_resume: bool, is_default: bool) -> anyhow::Result<()>
```

All existing callers pass `false` for `is_default` unless creating the default session.

**Step 4: Rewrite `complete_current_session`**

Replace the existing method with a two-phase approach:

```rust
/// Phase 1 of complete: merge code, clean worktrees
pub fn complete_session_merge(&mut self) -> anyhow::Result<bool> {
    let session = match &self.current_session {
        Some(s) => s.clone(),
        None => return Ok(false),
    };
    let project_path = match self.project_path.as_ref() {
        Some(p) => p.clone(),
        None => return Ok(false),
    };

    // Merge all branches to default git branch
    let default_branch = crate::worktree::default_branch(&project_path);
    let _ = std::process::Command::new("git")
        .args(["checkout", &default_branch])
        .current_dir(&project_path)
        .output();

    // For default session, only merge workers (no Leader branch)
    let pane_labels: Vec<String> = if session.is_default {
        (1..=session.worker_count).map(|i| format!("Worker {}", i)).collect()
    } else {
        std::iter::once("Leader".to_string())
            .chain((1..=session.worker_count).map(|i| format!("Worker {}", i)))
            .collect()
    };

    for label in &pane_labels {
        if let Err(e) = crate::worktree::merge_branch(&project_path, &session.name, label) {
            tracing::error!("Merge failed for {}: {}", label, e);
            return Err(e);
        }
    }

    // Remove worktrees
    if session.is_default {
        crate::worktree::remove_default_session_worktrees(
            &project_path, &session.name, session.worker_count as u16, false,
        )?;
    } else {
        crate::worktree::remove_session_worktrees(
            &project_path, &session.name, session.worker_count as u16, false,
        )?;
    }

    Ok(true)
}

/// Phase 2 of complete: handle records then delete session
pub fn complete_session_records(&mut self, migrate: bool) -> anyhow::Result<()> {
    let session = match self.current_session.take() {
        Some(s) => s,
        None => return Ok(()),
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    if let Ok(repo) = legion_db::open_db() {
        if migrate {
            // Find default session name
            let project_path = session.project_path.clone();
            if let Ok(Some(default_sess)) = repo.get_default_squad_session(&project_path) {
                repo.migrate_tickets_to_session(&session.name, &default_sess.name)?;
            }
        } else {
            repo.delete_session_tickets(&session.name)?;
        }

        // Mark completed (preserved in history)
        repo.complete_squad_session(&session.name, now)?;
    }

    Ok(())
}

/// Delete a session entirely (all data + worktrees + DB record)
pub fn delete_session(&mut self, session_name: &str) -> anyhow::Result<()> {
    let project_path = self.project_path.as_ref()
        .ok_or_else(|| anyhow::anyhow!("No project path"))?
        .clone();

    // Look up session info
    let session = if let Ok(repo) = legion_db::open_db() {
        repo.get_squad_session(session_name).ok().flatten()
    } else {
        None
    };

    let worker_count = session.as_ref().map(|s| s.worker_count as u16).unwrap_or(0);
    let is_default = session.as_ref().map(|s| s.is_default).unwrap_or(false);

    if is_default {
        anyhow::bail!("Cannot delete default session");
    }

    // Remove worktrees + branches
    crate::worktree::remove_session_worktrees(
        &project_path, session_name, worker_count, true,
    )?;

    // Delete all DB data
    if let Ok(repo) = legion_db::open_db() {
        repo.delete_session_tickets(session_name)?;
        repo.delete_squad_session(session_name)?;
    }

    // If deleting the current session, clear it
    if self.current_session.as_ref().map(|s| s.name.as_str()) == Some(session_name) {
        self.current_session = None;
    }

    Ok(())
}
```

**Step 5: Update `default_session_name` to use git branch name**

```rust
pub fn default_session_name_for_default(&self) -> String {
    if let Some(ref project_path) = self.project_path {
        crate::worktree::default_branch(project_path)
    } else {
        "main".to_string()
    }
}
```

**Step 6: Add popup sizing for new popups**

In the popup sizing logic (wherever `RetryForm`, `DeleteConfirm` etc. are sized), add:
```rust
PopupMenu::SessionDeleteConfirm => (55, 40),
PopupMenu::CompleteRecordChoice => (55, 30),
```

**Step 7: Commit**

```bash
git add crates/legion-tui/src/app.rs
git commit -m "feat(app): default session lifecycle, delete/complete with record handling"
```

---

### Task 4: Startup flow — Auto-resume default session

**Files:**
- Modify: `crates/legion-tui/src/lib.rs`

**Step 1: Update `run_squad` startup logic**

Replace the current session selection logic:

```rust
// In run_squad(), replace the session popup logic:

// Check for default session
let has_default = if let Ok(repo) = legion_db::open_db() {
    let pp = app.project_path.as_ref().unwrap().to_string_lossy().to_string();
    repo.get_default_squad_session(&pp).ok().flatten()
} else {
    None
};

if let Some(default_session) = has_default {
    // Auto-resume default session
    let workers = default_session.worker_count as u16;
    match app.start_session(&default_session.name, workers, true, true) {
        Ok(()) => {
            tracing::info!("Auto-resumed default session: {}", default_session.name);
            app.mode = app::AppMode::Normal;
        }
        Err(e) => {
            tracing::error!("Failed to resume default session: {}", e);
            // Fallback: show new session input
            app.mode = app::AppMode::Popup(app::PopupMenu::NewSessionInput);
            app.session_name_input = app.default_session_name_for_default();
        }
    }
} else {
    // No default session — show first-time setup
    app.mode = app::AppMode::Popup(app::PopupMenu::NewSessionInput);
    app.session_name_input = app.default_session_name_for_default();
    // Mark that this will create the default session
    app.creating_default_session = true;
}
```

Add `pub creating_default_session: bool` to App struct (initialized false).

**Step 2: Commit**

```bash
git add crates/legion-tui/src/lib.rs crates/legion-tui/src/app.rs
git commit -m "feat(startup): auto-resume default session, first-time setup flow"
```

---

### Task 5: Input — Session list keyboard shortcuts (d/c/n/x)

**Files:**
- Modify: `crates/legion-tui/src/input.rs`

**Step 1: Update `handle_session_list_keys`**

Add new key handlers:

```rust
fn handle_session_list_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            if !app.panes.is_empty() {
                app.mode = AppMode::Popup(PopupMenu::Main);
            }
        }
        KeyCode::Up | KeyCode::Char('k') => app.menu_up(),
        KeyCode::Down | KeyCode::Char('j') => app.menu_down(),
        KeyCode::Enter => {
            // Only resume active sessions
            if app.session_list_index < app.session_list.len() {
                let session = &app.session_list[app.session_list_index];
                if session.status == "active" {
                    let name = session.name.clone();
                    let workers = session.worker_count as u16;
                    let is_default = session.is_default;
                    match app.start_session(&name, workers, true, is_default) {
                        Ok(()) => {
                            tracing::info!("Resumed session: {}", name);
                            update_proxy_config(app);
                            app.mode = AppMode::Normal;
                        }
                        Err(e) => tracing::error!("Failed to resume session '{}': {}", name, e),
                    }
                }
            } else {
                // "New Session" — show text input
                app.session_name_input = app.default_session_name();
                app.mode = AppMode::Popup(PopupMenu::NewSessionInput);
            }
        }
        KeyCode::Char('n') => {
            app.session_name_input = app.default_session_name();
            app.mode = AppMode::Popup(PopupMenu::NewSessionInput);
        }
        KeyCode::Char('d') => {
            // Delete — only for active non-default sessions
            if app.session_list_index < app.session_list.len() {
                let session = &app.session_list[app.session_list_index];
                if !session.is_default && session.status == "active" {
                    let name = session.name.clone();
                    // Gather stats for confirm popup
                    let (pending, tickets, logs) = if let Ok(repo) = legion_db::open_db() {
                        (
                            repo.count_pending_tickets(&name).unwrap_or(0),
                            repo.count_tickets(&name).unwrap_or(0),
                            repo.count_ticket_logs(&name).unwrap_or(0),
                        )
                    } else {
                        (0, 0, 0)
                    };
                    app.session_delete_target = Some(name);
                    app.session_delete_pending_count = pending;
                    app.session_delete_ticket_count = tickets;
                    app.session_delete_log_count = logs;
                    app.mode = AppMode::Popup(PopupMenu::SessionDeleteConfirm);
                }
            }
        }
        KeyCode::Char('c') => {
            // Complete — only for active non-default sessions
            if app.session_list_index < app.session_list.len() {
                let session = &app.session_list[app.session_list_index];
                if !session.is_default && session.status == "active" {
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
                        let _ = repo.delete_squad_session(&name);
                    }
                    app.load_session_list();
                    if app.session_list_index >= app.session_list.len() && app.session_list_index > 0 {
                        app.session_list_index -= 1;
                    }
                }
            }
        }
        _ => {}
    }
}
```

**Step 2: Add `handle_session_delete_confirm_keys`**

```rust
fn handle_session_delete_confirm_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Enter | KeyCode::Char('y') => {
            if let Some(ref name) = app.session_delete_target.clone() {
                match app.delete_session(name) {
                    Ok(()) => tracing::info!("Deleted session: {}", name),
                    Err(e) => tracing::error!("Failed to delete session '{}': {}", name, e),
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
```

**Step 3: Update `handle_complete_session_keys` for new flow**

After merge succeeds, show record choice popup instead of quitting:

```rust
fn handle_complete_session_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.complete_session_name = None;
            app.mode = AppMode::Popup(PopupMenu::SessionList);
        }
        KeyCode::Up | KeyCode::Char('k') => app.menu_up(),
        KeyCode::Down | KeyCode::Char('j') => app.menu_down(),
        KeyCode::Enter => {
            // "merge" is the only strategy now (complete = merge + handle records)
            app.kill_all();

            match app.complete_session_merge() {
                Ok(true) => {
                    // Show record choice popup
                    app.complete_record_choice = 0;
                    app.mode = AppMode::Popup(PopupMenu::CompleteRecordChoice);
                }
                Ok(false) => {
                    tracing::warn!("No active session to complete");
                    app.mode = AppMode::Popup(PopupMenu::SessionList);
                }
                Err(e) => {
                    tracing::error!("Failed to merge session: {}", e);
                    app.mode = AppMode::Popup(PopupMenu::SessionList);
                }
            }
        }
        _ => {}
    }
}
```

**Step 4: Add `handle_complete_record_choice_keys`**

```rust
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
                Ok(()) => tracing::info!("Session completed (records {})", if migrate { "migrated" } else { "deleted" }),
                Err(e) => tracing::error!("Failed to handle session records: {}", e),
            }
            app.complete_session_name = None;
            // Reload session list and go back
            app.load_session_list();
            app.session_list_index = 0;
            app.mode = AppMode::Popup(PopupMenu::SessionList);
        }
        KeyCode::Esc => {
            // Can't cancel here — merge already done. Must choose.
            // Do nothing on Esc, force a choice.
        }
        _ => {}
    }
}
```

**Step 5: Update `handle_new_session_input_keys` for default session creation**

When `app.creating_default_session` is true, pass `is_default: true`:

```rust
KeyCode::Enter => {
    let name = app.session_name_input.trim().to_string();
    if !name.is_empty() {
        let workers = app.requested_workers;
        let is_default = app.creating_default_session;
        match app.start_session(&name, workers, false, is_default) {
            Ok(()) => {
                tracing::info!("Created {} session: {}", if is_default { "default" } else { "new" }, name);
                app.session_name_input.clear();
                app.creating_default_session = false;
                update_proxy_config(app);
                app.mode = AppMode::Normal;
            }
            Err(e) => {
                tracing::error!("Failed to create session '{}': {}", name, e);
            }
        }
    }
}
```

**Step 6: Wire popup dispatch in `handle_key`**

In the popup dispatch match, add:
```rust
PopupMenu::SessionDeleteConfirm => handle_session_delete_confirm_keys(app, key),
PopupMenu::CompleteRecordChoice => handle_complete_record_choice_keys(app, key),
```

**Step 7: Commit**

```bash
git add crates/legion-tui/src/input.rs
git commit -m "feat(input): session list d/c/n/x keys, delete confirm, complete record choice"
```

---

### Task 6: UI — Render session list sections and new popups

**Files:**
- Modify: `crates/legion-tui/src/ui.rs`

**Step 1: Update session list rendering**

The session list popup needs two sections (Active / Completed) with `[default]` tag. Find the existing `draw_session_list` or session list rendering code and update:

- Sort: Active sessions first (default on top), then completed
- Active sessions: show name, worker count, `[default]` tag if applicable
- Completed sessions: show name, completed date, grayed out
- Highlight the cursor row with selection style
- Footer: `[Enter=Resume] [n=New] [d=Delete] [c=Complete] [x=Remove]`

```rust
fn draw_session_list(frame: &mut Frame, app: &App, area: Rect) {
    let mut items: Vec<ListItem> = Vec::new();

    // Active sessions header
    items.push(ListItem::new(Span::styled("  Active:", Style::default().fg(Color::White).add_modifier(Modifier::BOLD))));

    let active: Vec<_> = app.session_list.iter()
        .filter(|s| s.status == "active")
        .collect();
    // Sort: default first
    let mut active_sorted = active.clone();
    active_sorted.sort_by(|a, b| b.is_default.cmp(&a.is_default));

    for sess in &active_sorted {
        let tag = if sess.is_default { " [default]" } else { "" };
        let line = format!("    {}{:>width$} workers",
            sess.name, sess.worker_count,
            width = 20usize.saturating_sub(sess.name.len() + tag.len()));
        let mut spans = vec![
            Span::raw(format!("    {}", sess.name)),
        ];
        if sess.is_default {
            spans.push(Span::styled(" [default]", Style::default().fg(Color::Cyan)));
        }
        spans.push(Span::styled(
            format!("  {} workers", sess.worker_count),
            Style::default().fg(Color::DarkGray),
        ));
        items.push(ListItem::new(Line::from(spans)));
    }

    // Completed sessions header
    let completed: Vec<_> = app.session_list.iter()
        .filter(|s| s.status == "completed")
        .collect();
    if !completed.is_empty() {
        items.push(ListItem::new("")); // spacer
        items.push(ListItem::new(Span::styled("  Completed:", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD))));
        for sess in &completed {
            let date = format_timestamp(sess.completed_at.unwrap_or(sess.created_at));
            items.push(ListItem::new(Span::styled(
                format!("    {}    {}", sess.name, date),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }

    // "New Session" entry at end
    items.push(ListItem::new(""));
    items.push(ListItem::new(Span::styled("  + New Session", Style::default().fg(Color::Green))));

    // Render with selection highlight
    // ... (use app.session_list_index to highlight the correct row)
}
```

Add a helper:
```rust
fn format_timestamp(ts: i64) -> String {
    // Simple date formatting from unix timestamp
    let secs = ts as u64;
    let days_since_epoch = secs / 86400;
    // Approximate: just show "N days ago" or use a simple formatter
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let diff = now.saturating_sub(secs);
    if diff < 86400 { "today".to_string() }
    else if diff < 172800 { "yesterday".to_string() }
    else { format!("{}d ago", diff / 86400) }
}
```

**Step 2: Add `draw_session_delete_confirm`**

```rust
fn draw_session_delete_confirm(frame: &mut Frame, app: &App, area: Rect) {
    let name = app.session_delete_target.as_deref().unwrap_or("?");

    let mut lines = vec![
        Line::from(Span::styled(
            format!("Delete session \"{}\"?", name),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    if app.session_delete_pending_count > 0 {
        lines.push(Line::from(Span::styled(
            format!("  ⚠ {} pending tasks!", app.session_delete_pending_count),
            Style::default().fg(Color::Yellow),
        )));
        lines.push(Line::from(""));
    }

    lines.push(Line::from("  The following will be removed:"));
    lines.push(Line::from("  • Git worktrees and branches"));
    if app.session_delete_ticket_count > 0 {
        lines.push(Line::from(format!("  • {} tickets", app.session_delete_ticket_count)));
    }
    if app.session_delete_log_count > 0 {
        lines.push(Line::from(format!("  • {} log entries", app.session_delete_log_count)));
    }
    lines.push(Line::from("  • Session record"));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  This action cannot be undone.",
        Style::default().fg(Color::Red),
    )));
    lines.push(Line::from(""));
    lines.push(Line::from("  [Enter/y = Delete]  [Esc = Cancel]"));

    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Delete Session "));
    frame.render_widget(paragraph, area);
}
```

**Step 3: Add `draw_complete_record_choice`**

```rust
fn draw_complete_record_choice(frame: &mut Frame, app: &App, area: Rect) {
    let options = ["Delete all records", "Migrate to default session"];
    let mut lines = vec![
        Line::from(Span::styled("Code merged successfully.", Style::default().fg(Color::Green))),
        Line::from(""),
        Line::from("What to do with task records?"),
        Line::from(""),
    ];

    for (i, opt) in options.iter().enumerate() {
        let prefix = if i == app.complete_record_choice { "> " } else { "  " };
        let style = if i == app.complete_record_choice {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(format!("{}{}", prefix, opt), style)));
    }

    lines.push(Line::from(""));
    lines.push(Line::from("  [Enter = Confirm]"));

    let paragraph = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" Complete Session "));
    frame.render_widget(paragraph, area);
}
```

**Step 4: Wire new popups in `draw_popup`**

In the popup content rendering match:
```rust
PopupMenu::SessionDeleteConfirm => draw_session_delete_confirm(frame, app, inner),
PopupMenu::CompleteRecordChoice => draw_complete_record_choice(frame, app, inner),
```

Add sizing:
```rust
PopupMenu::SessionDeleteConfirm => (55, 40),
PopupMenu::CompleteRecordChoice => (55, 25),
```

**Step 5: Commit**

```bash
git add crates/legion-tui/src/ui.rs
git commit -m "feat(ui): session list with Active/Completed sections, delete confirm, record choice popups"
```

---

### Task 7: Fix all callers — Update `start_session` and `SquadSession` construction

**Files:**
- Modify: All files that construct `SquadSession` or call `start_session`/`create_session`

**Step 1: Search and fix all `SquadSession { ... }` constructions**

Every `SquadSession` construction needs `is_default` field. Search for `SquadSession {` across the codebase. Add `is_default: false` (or appropriate value) to each.

**Step 2: Fix all `start_session(` calls**

Add the 4th `is_default` parameter. Search for `start_session(` and add `false` to all calls except the default session auto-resume path.

**Step 3: Fix all `create_session(` calls**

Add the 3rd `is_default` parameter. Search for `create_session(` and add `false`.

**Step 4: Fix all `TicketRow { ... }` constructions**

Add `origin_session: None` to each.

**Step 5: Compile and test**

Run: `cargo build`
Expected: Clean compilation

Run: `cargo test -p legion-db`
Expected: All tests pass

**Step 6: Commit**

```bash
git add -A
git commit -m "fix: update all SquadSession/TicketRow/start_session callers for new fields"
```

---

### Task 8: Integration test and cleanup

**Step 1: Run full test suite**

Run: `cargo test`
Expected: All tests pass

**Step 2: Manual smoke test checklist**

- [ ] `cargo run -- squad --workers 2` — first startup shows default session name input pre-filled with git branch name
- [ ] Enter creates default session, Leader uses main repo (no worktree created for Leader)
- [ ] Restart → auto-resumes default session
- [ ] Ctrl+P > Switch Session → shows session list with `[default]` tag
- [ ] Create new session from list (`n`)
- [ ] Delete non-default session from list (`d`) → shows confirm with ticket/log counts
- [ ] Complete non-default session (`c`) → merges code → shows record choice (delete/migrate)
- [ ] Completed sessions appear in bottom section, `x` removes from history
- [ ] Default session cannot be deleted or completed (keys disabled)

**Step 3: Final commit**

```bash
git add -A
git commit -m "feat: default session with auto-resume, session delete/complete lifecycle"
```
