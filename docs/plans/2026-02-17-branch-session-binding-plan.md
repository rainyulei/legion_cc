# Branch-Session Binding Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Bind sessions to git branches with DB persistence, guided first-launch flow, branch-deleted detection/recovery, runtime branch monitoring, and manual branch switching.

**Architecture:** Add `base_branch` + `base_commit` columns to `squad_sessions`. On first launch, auto-detect current branch and confirm. On resume, verify branch still exists; if deleted, show recovery dialog. Periodically check for branch changes at runtime. Add "Switch Branch" to main menu.

**Tech Stack:** Rust, rusqlite (ALTER TABLE migration), ratatui TUI popups, git CLI for branch detection.

---

### Task 1: DB Schema Migration — Add base_branch + base_commit

**Files:**
- Modify: `crates/legion-db/src/schema.rs:104-107`
- Modify: `crates/legion-db/src/repo.rs:25-34` (SquadSession struct)
- Modify: `crates/legion-db/src/repo.rs:298-312` (upsert_squad_session)
- Modify: `crates/legion-db/src/repo.rs:314-332` (get_squad_session)
- Modify: `crates/legion-db/src/repo.rs:334-350` (list_squad_sessions)
- Modify: `crates/legion-db/src/repo.rs:352-368` (list_active_squad_sessions)
- Modify: `crates/legion-db/src/repo.rs:503-521` (get_default_squad_session)

**Step 1: Add migration to schema.rs**

In `init_db()`, after `conn.execute_batch(SCHEMA)?;`, add ALTER TABLE migrations (using `execute_batch` that ignores "duplicate column" errors):

```rust
pub fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
    // Migrations — safe to re-run (ignore "duplicate column" errors)
    let _ = conn.execute("ALTER TABLE squad_sessions ADD COLUMN base_branch TEXT", []);
    let _ = conn.execute("ALTER TABLE squad_sessions ADD COLUMN base_commit TEXT", []);
    Ok(())
}
```

**Step 2: Update SquadSession struct**

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
    pub base_branch: Option<String>,
    pub base_commit: Option<String>,
}
```

**Step 3: Update all SQL queries in repo.rs**

Every SELECT must now include `base_branch, base_commit` (columns 7, 8). Every INSERT must include them. Update these 5 methods:

- `upsert_squad_session`: INSERT column list adds `base_branch, base_commit`, params add `session.base_branch, session.base_commit`
- `get_squad_session`: SELECT adds columns, struct construction adds `base_branch: row.get(7)?, base_commit: row.get(8)?`
- `list_squad_sessions`: same pattern
- `list_active_squad_sessions`: same pattern
- `get_default_squad_session`: same pattern

**Step 4: Build and run tests**

Run: `cargo build -p legion-db && cargo test -p legion-db`
Expected: PASS (existing tests compile with new Option fields defaulting)

**Step 5: Commit**

```
feat(db): add base_branch and base_commit columns to squad_sessions
```

---

### Task 2: Git Branch Detection Helpers

**Files:**
- Modify: `crates/legion-tui/src/worktree.rs` (add new functions)

**Step 1: Add `current_branch()` function**

```rust
/// Get the current branch name (None if detached HEAD)
pub fn current_branch(project_path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(project_path)
        .output()
        .ok()?;
    if output.status.success() {
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if branch.is_empty() { None } else { Some(branch) }
    } else {
        None
    }
}
```

**Step 2: Add `current_commit()` function**

```rust
/// Get the current HEAD commit SHA (short)
pub fn current_commit(project_path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(project_path)
        .output()
        .ok()?;
    if output.status.success() {
        let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if sha.is_empty() { None } else { Some(sha) }
    } else {
        None
    }
}
```

**Step 3: Add `branch_exists()` function**

```rust
/// Check if a local branch exists
pub fn branch_exists(project_path: &Path, branch: &str) -> bool {
    Command::new("git")
        .args(["rev-parse", "--verify", branch])
        .current_dir(project_path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
```

**Step 4: Add `list_local_branches()` function**

```rust
/// List all local branch names
pub fn list_local_branches(project_path: &Path) -> Vec<String> {
    let output = Command::new("git")
        .args(["branch", "--format=%(refname:short)"])
        .current_dir(project_path)
        .output();
    match output {
        Ok(o) if o.status.success() => {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect()
        }
        _ => vec![],
    }
}
```

**Step 5: Add `sanitize_branch_name()` function**

```rust
/// Sanitize a branch name for use as session name (replace / with -)
pub fn sanitize_branch_name(branch: &str) -> String {
    branch.replace('/', "-")
}
```

**Step 6: Build**

Run: `cargo build -p legion-tui`
Expected: PASS

**Step 7: Commit**

```
feat(worktree): add git branch detection helpers
```

---

### Task 3: First-Launch Guided Flow — Auto-detect Branch + Confirmation

**Files:**
- Modify: `crates/legion-tui/src/app.rs:23-42` (PopupMenu enum)
- Modify: `crates/legion-tui/src/app.rs:230-243` (App fields)
- Modify: `crates/legion-tui/src/lib.rs:86-113` (startup flow)
- Modify: `crates/legion-tui/src/input.rs:260` (key dispatch)
- Modify: `crates/legion-tui/src/input.rs:430-469` (handle_new_session_input_keys)
- Modify: `crates/legion-tui/src/ui.rs:705,747` (popup dispatch)
- Modify: `crates/legion-tui/src/ui.rs:1160-1189` (draw_new_session_input)

**Step 1: Add App fields for branch state**

In App struct (around line 241), add:

```rust
pub detected_branch: Option<String>,   // current HEAD branch at startup
pub detected_commit: Option<String>,   // current HEAD commit SHA at startup
```

Initialize both as `None` in App::new().

**Step 2: Update startup flow in lib.rs**

Replace lines 108-113 (the `else` branch — no default session):

```rust
    } else {
        // No default session — first-time setup with branch detection
        if let Some(ref project_path) = app.project_path {
            app.detected_branch = crate::worktree::current_branch(project_path);
            app.detected_commit = crate::worktree::current_commit(project_path);
        }
        if app.detected_branch.is_some() {
            // Auto-fill session name from branch
            let branch = app.detected_branch.as_ref().unwrap();
            app.session_name_input = crate::worktree::sanitize_branch_name(branch);
        } else {
            app.session_name_input = app.default_session_name_for_default();
        }
        app.mode = app::AppMode::Popup(app::PopupMenu::NewSessionInput);
        app.creating_default_session = true;
    }
```

**Step 3: Update draw_new_session_input to show branch info**

Modify `draw_new_session_input()` in ui.rs to show detected branch:

```rust
fn draw_new_session_input(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" New Session [ESC] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Green))
        .style(Style::default().bg(Color::DarkGray));

    let mut items = vec![];

    // Show branch info if detected
    if let Some(ref branch) = app.detected_branch {
        items.push(ListItem::new(Line::from(vec![
            Span::styled("  Branch: ", Style::default().fg(Color::DarkGray)),
            Span::styled(branch.as_str(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ])));
        if let Some(ref commit) = app.detected_commit {
            items.push(ListItem::new(Line::from(vec![
                Span::styled("  Commit: ", Style::default().fg(Color::DarkGray)),
                Span::styled(commit.as_str(), Style::default().fg(Color::DarkGray)),
            ])));
        }
        items.push(ListItem::new(Line::from(Span::raw(""))));
    }

    items.push(ListItem::new(Line::from(Span::styled(
        "  Enter session name:",
        Style::default().fg(Color::White),
    ))));
    items.push(ListItem::new(Line::from(Span::raw(""))));
    items.push(ListItem::new(Line::from(vec![
        Span::styled("  > ", Style::default().fg(Color::Yellow)),
        Span::styled(
            app.session_name_input.as_str(),
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        ),
        Span::styled("\u{2588}", Style::default().fg(Color::Yellow)),
    ])));
    items.push(ListItem::new(Line::from(Span::raw(""))));

    let workers_hint = format!("  {} workers  |  [Enter=Create] [Esc=Cancel]", app.requested_workers);
    items.push(ListItem::new(Line::from(Span::styled(
        workers_hint,
        Style::default().fg(Color::DarkGray),
    ))));

    frame.render_widget(List::new(items).block(block), area);
}
```

**Step 4: Pass branch info to create_session**

Modify `create_session()` in app.rs to populate `base_branch` and `base_commit`:

```rust
    let session = SquadSession {
        name: name.to_string(),
        project_path: project_path.to_string_lossy().to_string(),
        worker_count: worker_count as i64,
        status: "active".to_string(),
        created_at: now,
        completed_at: None,
        is_default,
        base_branch: self.detected_branch.clone(),
        base_commit: self.detected_commit.clone(),
    };
```

Also detect branch on demand if `detected_branch` is None (for non-startup session creation):

Add before the `SquadSession` construction:

```rust
    // Detect branch if not already set (non-startup creation)
    if self.detected_branch.is_none() {
        self.detected_branch = crate::worktree::current_branch(project_path);
        self.detected_commit = crate::worktree::current_commit(project_path);
    }
```

**Step 5: Build and test**

Run: `cargo build -p legion-tui && cargo test`
Expected: PASS

**Step 6: Commit**

```
feat(tui): guided first-launch session creation with branch detection
```

---

### Task 4: Session List — Show Branch Status

**Files:**
- Modify: `crates/legion-tui/src/app.rs` (add `session_branch_status` field, branch check on load)
- Modify: `crates/legion-tui/src/ui.rs:1036-1147` (draw_session_list)

**Step 1: Add branch status cache to App**

```rust
/// Branch status for session list display
pub session_branch_status: HashMap<String, bool>, // session_name → branch_exists
```

Initialize as `HashMap::new()` in App::new().

**Step 2: Populate branch status in load_session_list()**

Find `load_session_list()` method. After loading sessions, check branch status:

```rust
    // Check branch status for all sessions
    self.session_branch_status.clear();
    if let Some(ref project_path) = self.project_path {
        for session in &self.session_list {
            if let Some(ref branch) = session.base_branch {
                let exists = crate::worktree::branch_exists(project_path, branch);
                self.session_branch_status.insert(session.name.clone(), exists);
            }
        }
    }
```

**Step 3: Update draw_session_list to show branch info**

In the active sessions loop (around line 1075-1087), after the worker count span, add branch display:

```rust
    // Branch info
    if let Some(ref branch) = session.base_branch {
        let branch_exists = app.session_branch_status.get(&session.name).copied().unwrap_or(true);
        if branch_exists {
            spans.push(Span::styled(
                format!(" \u{2190} {}", branch), // ← branch
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            spans.push(Span::styled(
                format!(" \u{26a0} branch '{}' deleted", branch), // ⚠ branch deleted
                Style::default().fg(Color::Red),
            ));
            if let Some(ref commit) = session.base_commit {
                spans.push(Span::styled(
                    format!(" ({})", commit),
                    Style::default().fg(Color::DarkGray),
                ));
            }
        }
    }
```

**Step 4: Build and test**

Run: `cargo build -p legion-tui && cargo test`
Expected: PASS

**Step 5: Commit**

```
feat(tui): show branch status in session list with deleted warning
```

---

### Task 5: Branch-Deleted Recovery Dialog

**Files:**
- Modify: `crates/legion-tui/src/app.rs:23-42` (PopupMenu enum — add BranchRecovery)
- Modify: `crates/legion-tui/src/app.rs` (App fields for recovery state)
- Modify: `crates/legion-tui/src/input.rs:381-401` (intercept resume if branch deleted)
- Create: recovery dialog handler in `input.rs`
- Modify: `crates/legion-tui/src/ui.rs` (draw_branch_recovery popup)

**Step 1: Add PopupMenu variant and App fields**

```rust
// In PopupMenu enum:
BranchRecovery,
BranchList,  // for "Select another branch" sub-dialog

// In App struct:
pub recovery_session: Option<SquadSession>,  // session being recovered
pub recovery_choice: usize,                   // 0-3 selected option
pub branch_list: Vec<String>,                 // cached local branches
pub branch_list_index: usize,                 // selected branch in list
```

**Step 2: Intercept session resume in handle_session_list_keys**

In the `KeyCode::Enter` branch (line 381-401), before calling `start_session`, check if branch is deleted:

```rust
    // Resume existing session
    let session = app.session_list[app.session_list_index].clone();

    // Check if branch was deleted
    if let (Some(ref branch), Some(ref project_path)) = (&session.base_branch, &app.project_path) {
        if !crate::worktree::branch_exists(project_path, branch) {
            // Show recovery dialog
            app.recovery_session = Some(session);
            app.recovery_choice = 0;
            app.mode = AppMode::Popup(PopupMenu::BranchRecovery);
            return;
        }
    }

    // Normal resume (branch exists or no branch info)
    let workers = session.worker_count as u16;
    // ... existing resume code ...
```

**Step 3: Handle recovery dialog keys**

Add `handle_branch_recovery_keys()`:

```rust
fn handle_branch_recovery_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.recovery_session = None;
            app.mode = AppMode::Popup(PopupMenu::SessionList);
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
                    // Bind to current branch
                    if let Some(ref project_path) = app.project_path {
                        app.detected_branch = crate::worktree::current_branch(project_path);
                        app.detected_commit = crate::worktree::current_commit(project_path);
                    }
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
                3 => {
                    // Cancel
                    app.mode = AppMode::Popup(PopupMenu::SessionList);
                }
                _ => {}
            }
        }
        _ => {}
    }
}
```

Helper functions:

```rust
fn update_session_branch(app: &App, session_name: &str) {
    if let Ok(repo) = legion_db::open_db() {
        if let Ok(Some(mut session)) = repo.get_squad_session(session_name) {
            session.base_branch = app.detected_branch.clone();
            session.base_commit = app.detected_commit.clone();
            let _ = repo.upsert_squad_session(&session);
        }
    }
}

fn resume_session(app: &mut App, session: &SquadSession) {
    let workers = session.worker_count as u16;
    let is_default = session.is_default;
    match app.start_session(&session.name, workers, true, is_default) {
        Ok(()) => {
            tracing::info!("Resumed session: {}", session.name);
            update_proxy_config(app);
            app.mode = AppMode::Normal;
        }
        Err(e) => {
            tracing::error!("Failed to resume session '{}': {}", session.name, e);
            app.mode = AppMode::Popup(PopupMenu::SessionList);
        }
    }
}
```

**Step 4: Handle branch list dialog keys**

Add `handle_branch_list_keys()`:

```rust
fn handle_branch_list_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            // Back to recovery dialog
            app.recovery_choice = 1;
            app.mode = AppMode::Popup(PopupMenu::BranchRecovery);
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
                app.detected_branch = Some(branch);
                if let Some(ref project_path) = app.project_path {
                    app.detected_commit = crate::worktree::current_commit(project_path);
                }
                if let Some(session) = app.recovery_session.take() {
                    update_session_branch(app, &session.name);
                    resume_session(app, &session);
                }
            }
        }
        _ => {}
    }
}
```

**Step 5: Add popup dispatch in input.rs and UI rendering**

In `handle_key_event()` (input.rs), add:
```rust
AppMode::Popup(PopupMenu::BranchRecovery) => handle_branch_recovery_keys(app, key),
AppMode::Popup(PopupMenu::BranchList) => handle_branch_list_keys(app, key),
```

In ui.rs, add draw functions for both popups and wire into the popup dispatch.

**Step 6: Draw branch recovery popup**

```rust
fn draw_branch_recovery(frame: &mut Frame, app: &App, area: Rect) {
    let session = match &app.recovery_session {
        Some(s) => s,
        None => return,
    };
    let branch = session.base_branch.as_deref().unwrap_or("unknown");
    let commit = session.base_commit.as_deref().unwrap_or("unknown");

    let block = Block::default()
        .title(format!(" \u{26a0} Branch Deleted [ESC] "))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(Color::DarkGray));

    let current_branch = app.detected_branch.as_deref().unwrap_or("unknown");

    let options = [
        format!("Bind to current branch ({}) and continue", current_branch),
        "Select another branch".to_string(),
        format!("Create new branch from base commit ({})", commit),
        "Cancel".to_string(),
    ];

    let mut items = vec![
        ListItem::new(Line::from(Span::styled(
            format!("  Branch '{}' has been deleted", branch),
            Style::default().fg(Color::Yellow),
        ))),
        ListItem::new(Line::from(Span::styled(
            format!("  Base commit: {}", commit),
            Style::default().fg(Color::DarkGray),
        ))),
        ListItem::new(""),
    ];

    for (i, opt) in options.iter().enumerate() {
        let selected = i == app.recovery_choice;
        let prefix = if selected { "> " } else { "  " };
        let style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        items.push(ListItem::new(Line::from(Span::styled(
            format!("{}{}", prefix, opt),
            style,
        ))));
    }

    frame.render_widget(List::new(items).block(block), area);
}
```

**Step 7: Build and test**

Run: `cargo build -p legion-tui && cargo test`
Expected: PASS

**Step 8: Commit**

```
feat(tui): add branch-deleted recovery dialog with rebind/create options
```

---

### Task 6: Runtime Branch Change Detection

**Files:**
- Modify: `crates/legion-tui/src/app.rs` (add PopupMenu::BranchChanged, App fields)
- Modify: `crates/legion-tui/src/lib.rs` (event loop — periodic branch check)
- Modify: `crates/legion-tui/src/input.rs` (handle_branch_changed_keys)
- Modify: `crates/legion-tui/src/ui.rs` (draw_branch_changed popup)

**Step 1: Add App fields**

```rust
pub last_branch_check: Option<std::time::Instant>,  // when we last checked
pub branch_changed_to: Option<String>,               // new branch name detected
```

Add `BranchChanged` to `PopupMenu` enum.

**Step 2: Add periodic check in event loop**

In `run_event_loop()` in lib.rs, inside the main loop (after handling events, before sleep/poll), add:

```rust
    // Periodic branch check (every 5s)
    if app.mode == app::AppMode::Normal {
        let should_check = app.last_branch_check
            .map(|t| t.elapsed().as_secs() >= 5)
            .unwrap_or(true);
        if should_check {
            app.last_branch_check = Some(std::time::Instant::now());
            if let (Some(ref session), Some(ref project_path)) = (&app.current_session, &app.project_path) {
                if let Some(ref base_branch) = session.base_branch {
                    if let Some(current) = crate::worktree::current_branch(project_path) {
                        if &current != base_branch {
                            app.branch_changed_to = Some(current);
                            app.recovery_choice = 0;
                            app.mode = app::AppMode::Popup(app::PopupMenu::BranchChanged);
                        }
                    }
                }
            }
        }
    }
```

**Step 3: Handle branch changed dialog**

```rust
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
                    // TODO: rebuild worktrees (complex — may defer to later task)
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
                    // TODO: rebase worktrees (complex — may defer to later task)
                    app.mode = AppMode::Normal;
                }
                2 => {
                    // Ignore
                    app.mode = AppMode::Normal;
                }
                _ => {}
            }
        }
        _ => {}
    }
}
```

**Step 4: Draw branch changed popup**

```rust
fn draw_branch_changed(frame: &mut Frame, app: &App, area: Rect) {
    let old = app.current_session.as_ref()
        .and_then(|s| s.base_branch.as_deref())
        .unwrap_or("unknown");
    let new = app.branch_changed_to.as_deref().unwrap_or("unknown");

    let block = Block::default()
        .title(" Branch Changed [ESC=Ignore] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow))
        .style(Style::default().bg(Color::DarkGray));

    let options = [
        format!("Switch to '{}' (rebuild worktrees)", new),
        format!("Switch to '{}' (rebase worktrees)", new),
        "Ignore".to_string(),
    ];

    let mut items = vec![
        ListItem::new(Line::from(vec![
            Span::styled("  Branch changed: ", Style::default().fg(Color::Yellow)),
            Span::styled(old, Style::default().fg(Color::Red)),
            Span::styled(" \u{2192} ", Style::default().fg(Color::Yellow)),
            Span::styled(new, Style::default().fg(Color::Green)),
        ])),
        ListItem::new(""),
    ];

    for (i, opt) in options.iter().enumerate() {
        let selected = i == app.recovery_choice;
        let prefix = if selected { "> " } else { "  " };
        let style = if selected {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        items.push(ListItem::new(Line::from(Span::styled(format!("{}{}", prefix, opt), style))));
    }

    frame.render_widget(List::new(items).block(block), area);
}
```

**Step 5: Wire up dispatch in input.rs and ui.rs**

**Step 6: Build and test**

Run: `cargo build -p legion-tui && cargo test`
Expected: PASS

**Step 7: Commit**

```
feat(tui): runtime branch change detection with switch/ignore options
```

---

### Task 7: Manual Branch Switching via Main Menu

**Files:**
- Modify: `crates/legion-tui/src/app.rs` (MainMenuItem enum, main_menu_items, enter_submenu)
- Modify: `crates/legion-tui/src/input.rs` (handle branch switch from menu)
- Modify: `crates/legion-tui/src/ui.rs` (render menu item + branch list)

**Step 1: Add MainMenuItem::SwitchBranch**

```rust
// In MainMenuItem enum:
SwitchBranch,

// In label():
Self::SwitchBranch => "Switch Branch",
```

**Step 2: Add to main_menu_items()**

In `main_menu_items()`, after `MaxRetries` and before `SwitchSession`:

```rust
    if self.current_session.as_ref().map(|s| s.base_branch.is_some()).unwrap_or(false) {
        items.push(MainMenuItem::SwitchBranch);
    }
```

**Step 3: Handle in enter_submenu()**

```rust
    MainMenuItem::SwitchBranch => {
        if let Some(ref project_path) = self.project_path {
            self.branch_list = crate::worktree::list_local_branches(project_path);
        }
        self.branch_list_index = 0;
        self.mode = AppMode::Popup(PopupMenu::BranchList);
        // Set recovery_session to None — BranchList handler will check this
        // to know if it's a recovery flow or a manual switch
        self.recovery_session = None;
    }
```

**Step 4: Update handle_branch_list_keys for manual switch mode**

When `recovery_session` is None, we're in manual switch mode. On Enter, update session branch and prompt for worktree handling:

```rust
    KeyCode::Enter => {
        if app.branch_list_index < app.branch_list.len() {
            let branch = app.branch_list[app.branch_list_index].clone();
            app.detected_branch = Some(branch.clone());
            if let Some(ref project_path) = app.project_path {
                app.detected_commit = crate::worktree::current_commit(project_path);
            }

            if let Some(session) = app.recovery_session.take() {
                // Recovery flow
                update_session_branch(app, &session.name);
                resume_session(app, &session);
            } else {
                // Manual switch — update current session branch, show rebuild/rebase prompt
                if let Some(ref session) = app.current_session {
                    update_session_branch(app, &session.name);
                }
                app.branch_changed_to = Some(branch);
                app.recovery_choice = 0;
                app.mode = AppMode::Popup(PopupMenu::BranchChanged);
            }
        }
    }
```

**Step 5: Build and test**

Run: `cargo build -p legion-tui && cargo test`
Expected: PASS

**Step 6: Commit**

```
feat(tui): add Switch Branch option to main menu
```

---

### Task 8: Update Auto-Resume to Check Branch Status

**Files:**
- Modify: `crates/legion-tui/src/lib.rs:94-107` (auto-resume default session)

**Step 1: Add branch check before auto-resume**

Replace lines 94-107:

```rust
    if let Some(default_sess) = default_session {
        // Detect current branch
        if let Some(ref project_path) = app.project_path {
            app.detected_branch = crate::worktree::current_branch(project_path);
            app.detected_commit = crate::worktree::current_commit(project_path);
        }

        // Check if session's branch was deleted
        let branch_deleted = if let (Some(ref branch), Some(ref project_path)) = (&default_sess.base_branch, &app.project_path) {
            !crate::worktree::branch_exists(project_path, branch)
        } else {
            false
        };

        if branch_deleted {
            // Show recovery dialog instead of auto-resuming
            app.recovery_session = Some(default_sess);
            app.recovery_choice = 0;
            app.mode = app::AppMode::Popup(app::PopupMenu::BranchRecovery);
        } else {
            // Normal auto-resume
            let workers = default_sess.worker_count as u16;
            match app.start_session(&default_sess.name, workers, true, true) {
                Ok(()) => {
                    tracing::info!("Auto-resumed default session: {}", default_sess.name);
                    app.mode = app::AppMode::Normal;
                }
                Err(e) => {
                    tracing::error!("Failed to resume default session: {}", e);
                    app.mode = app::AppMode::Popup(app::PopupMenu::NewSessionInput);
                    app.session_name_input = app.default_session_name_for_default();
                }
            }
        }
    } else {
        // ... first-launch flow from Task 3 ...
    }
```

**Step 2: Build and test**

Run: `cargo build -p legion-tui && cargo test`
Expected: PASS

**Step 3: Commit**

```
feat(tui): check branch status on auto-resume, show recovery if deleted
```
