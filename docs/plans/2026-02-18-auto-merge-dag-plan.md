# Auto-Merge + Task DAG Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Worker 完成任务后自动 merge 代码到 leader branch，并通过 DAG 依赖控制任务执行顺序。

**Architecture:** 在 `TaskTicket` 中添加 `blocked_by` 和 `merge_status` 字段，修改 `take_next()` 调度逻辑加入 DAG 就绪检查，在 `lib.rs` 事件循环中添加 auto-merge（完成后）和 rebase-on-start（取任务前）逻辑。CLI 工具 `legion-dispatch` 新增 `--after` 参数。

**Tech Stack:** Rust, SQLite (rusqlite), git CLI, serde, hyper

---

## Task 1: Add MergeStatus enum and blocked_by/merge_status to data model

**Files:**
- Modify: `crates/legion-core/src/orchestrate/engine.rs:8-80` — Add MergeStatus enum, add fields to TaskTicket and TicketSnapshot
- Modify: `crates/legion-core/src/orchestrate/mod.rs:5` — Export MergeStatus
- Modify: `crates/legion-core/src/lib.rs:13` — Export MergeStatus

**Step 1: Add MergeStatus enum after TicketStatus**

In `crates/legion-core/src/orchestrate/engine.rs`, after line 15 (`}` closing TicketStatus), add:

```rust
/// Merge status of a completed ticket's code into the leader branch
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStatus {
    Pending,
    Merged,
    Conflict,
    Skipped,
}

impl Default for MergeStatus {
    fn default() -> Self { Self::Pending }
}
```

**Step 2: Add fields to TaskTicket**

In `TaskTicket` struct (after `base_commit` field at line 51), add:

```rust
    pub blocked_by: Vec<usize>,
    pub merge_status: MergeStatus,
```

**Step 3: Add fields to TicketSnapshot**

In `TicketSnapshot` struct (after `base_commit` field at line 79), add:

```rust
    pub blocked_by: Vec<usize>,
    pub merge_status: MergeStatus,
```

**Step 4: Update ticket_to_snapshot**

In `ticket_to_snapshot()` (line 514), add to the TicketSnapshot construction:

```rust
        blocked_by: t.blocked_by.clone(),
        merge_status: t.merge_status,
```

**Step 5: Update exports**

In `crates/legion-core/src/orchestrate/mod.rs`, change line 5 to:
```rust
pub use engine::{OrchestrateEngine, TicketSnapshot, TicketStatus, TeamMode, MergeStatus};
```

In `crates/legion-core/src/lib.rs`, change line 13 to:
```rust
pub use orchestrate::{OrchestrateApi, OrchestrateEngine, TicketSnapshot, TicketStatus, TeamMode, MergeStatus};
```

**Step 6: Fix all places that construct TaskTicket**

In `submit_ticket()` (line 266), add to the TaskTicket construction:
```rust
            blocked_by: Vec::new(),
            merge_status: MergeStatus::Pending,
```

In `with_db()` (line 140), add to the TaskTicket construction:
```rust
                        blocked_by: Vec::new(), // will be loaded from DB in Task 2
                        merge_status: MergeStatus::Pending, // will be loaded from DB in Task 2
                    });
```

**Step 7: Verify it compiles**

Run: `cargo build -p legion-core 2>&1 | head -20`
Expected: Compiles (possibly with warnings about unused fields)

**Step 8: Commit**

```bash
git add crates/legion-core/
git commit -m "feat(core): add MergeStatus enum and blocked_by/merge_status fields to TaskTicket"
```

---

## Task 2: Add DB schema migration and persistence for blocked_by/merge_status

**Files:**
- Modify: `crates/legion-db/src/schema.rs:113` — Add ALTER TABLE migrations
- Modify: `crates/legion-db/src/repo.rs:52-70` — Add fields to TicketRow
- Modify: `crates/legion-db/src/repo.rs:422-491` — Update insert/update/list SQL queries

**Step 1: Add DB migrations**

In `crates/legion-db/src/schema.rs`, after line 113 (`ALTER TABLE tickets ADD COLUMN base_commit`), add:

```rust
    let _ = conn.execute("ALTER TABLE tickets ADD COLUMN blocked_by TEXT DEFAULT '[]'", []);
    let _ = conn.execute("ALTER TABLE tickets ADD COLUMN merge_status TEXT DEFAULT 'pending'", []);
```

**Step 2: Add fields to TicketRow**

In `crates/legion-db/src/repo.rs`, in the `TicketRow` struct (after `base_commit` at line 69), add:

```rust
    pub blocked_by: String,        // JSON array e.g. "[1, 3]"
    pub merge_status: String,      // "pending" / "merged" / "conflict" / "skipped"
```

**Step 3: Update insert_ticket SQL**

In `insert_ticket()` (line 422), update the SQL and params to include the two new columns:

```rust
    pub fn insert_ticket(&self, ticket: &TicketRow) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO tickets (id, session_name, title, prompt, context, criteria, status, assigned_worker, team_mode, iteration, max_iterations, feedback, summary, created_at, updated_at, origin_session, base_commit, blocked_by, merge_status) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                ticket.id,
                ticket.session_name,
                ticket.title,
                ticket.prompt,
                ticket.context,
                ticket.criteria,
                ticket.status,
                ticket.assigned_worker,
                ticket.team_mode,
                ticket.iteration,
                ticket.max_iterations,
                ticket.feedback,
                ticket.summary,
                ticket.created_at,
                ticket.updated_at,
                ticket.origin_session,
                ticket.base_commit,
                ticket.blocked_by,
                ticket.merge_status,
            ],
        )?;
        Ok(())
    }
```

**Step 4: Update update_ticket SQL**

In `update_ticket()` (line 448), add merge_status to the UPDATE:

```rust
    pub fn update_ticket(&self, ticket: &TicketRow) -> Result<()> {
        self.conn.execute(
            "UPDATE tickets SET status = ?1, assigned_worker = ?2, iteration = ?3, feedback = ?4, summary = ?5, updated_at = ?6, base_commit = ?9, merge_status = ?10 WHERE id = ?7 AND session_name = ?8",
            params![
                ticket.status,
                ticket.assigned_worker,
                ticket.iteration,
                ticket.feedback,
                ticket.summary,
                ticket.updated_at,
                ticket.id,
                ticket.session_name,
                ticket.base_commit,
                ticket.merge_status,
            ],
        )?;
        Ok(())
    }
```

**Step 5: Update list_tickets_by_session SQL**

In `list_tickets_by_session()` (line 466), add the two new columns to SELECT and row mapping:

```rust
    pub fn list_tickets_by_session(&self, session_name: &str) -> Result<Vec<TicketRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_name, title, prompt, context, criteria, status, assigned_worker, team_mode, iteration, max_iterations, feedback, summary, created_at, updated_at, origin_session, base_commit, blocked_by, merge_status FROM tickets WHERE session_name = ? ORDER BY id"
        )?;
        let rows = stmt.query_map(params![session_name], |row| {
            Ok(TicketRow {
                id: row.get(0)?,
                session_name: row.get(1)?,
                title: row.get(2)?,
                prompt: row.get(3)?,
                context: row.get(4)?,
                criteria: row.get(5)?,
                status: row.get(6)?,
                assigned_worker: row.get(7)?,
                team_mode: row.get(8)?,
                iteration: row.get(9)?,
                max_iterations: row.get(10)?,
                feedback: row.get(11)?,
                summary: row.get(12)?,
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
                origin_session: row.get(15)?,
                base_commit: row.get(16)?,
                blocked_by: row.get::<_, Option<String>>(17)?.unwrap_or_else(|| "[]".to_string()),
                merge_status: row.get::<_, Option<String>>(18)?.unwrap_or_else(|| "pending".to_string()),
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
```

Note: `unwrap_or_else` handles rows created before the migration (column might be NULL).

**Step 6: Update engine's with_db() to load blocked_by/merge_status from DB**

In `crates/legion-core/src/orchestrate/engine.rs`, in `with_db()` (around line 140), replace the placeholder `blocked_by: Vec::new()` and `merge_status: MergeStatus::Pending` with:

```rust
                        blocked_by: serde_json::from_str(&row.blocked_by).unwrap_or_default(),
                        merge_status: match row.merge_status.as_str() {
                            "merged" => MergeStatus::Merged,
                            "conflict" => MergeStatus::Conflict,
                            "skipped" => MergeStatus::Skipped,
                            _ => MergeStatus::Pending,
                        },
```

Note: `row.blocked_by` is a `String` field added to `TicketRow` in Step 2. You'll need to add `blocked_by` and `merge_status` to the `TicketRow` construction in `with_db()` accordingly — but `with_db()` reads `TicketRow` from `list_tickets_by_session`, which already returns the new fields from Step 5.

**Step 7: Update engine's persist methods to serialize blocked_by/merge_status**

In `persist_ticket()` and `persist_ticket_update()`, add the new fields to the TicketRow construction:

```rust
                blocked_by: serde_json::to_string(&ticket.blocked_by).unwrap_or_else(|_| "[]".into()),
                merge_status: match ticket.merge_status {
                    MergeStatus::Pending => "pending".to_string(),
                    MergeStatus::Merged => "merged".to_string(),
                    MergeStatus::Conflict => "conflict".to_string(),
                    MergeStatus::Skipped => "skipped".to_string(),
                },
```

Add this to **both** `persist_ticket()` (line 195 area) and `persist_ticket_update()` (line 234 area) in the TicketRow construction.

**Step 8: Fix test TicketRow constructions**

Search for any tests in `crates/legion-db/src/repo.rs` that construct `TicketRow` and add the new fields:
```rust
                blocked_by: "[]".to_string(),
                merge_status: "pending".to_string(),
```

**Step 9: Verify it compiles and tests pass**

Run: `cargo build -p legion-db -p legion-core 2>&1 | head -20`
Run: `cargo test -p legion-db 2>&1 | tail -20`
Expected: All pass

**Step 10: Commit**

```bash
git add crates/legion-db/ crates/legion-core/
git commit -m "feat(db): add blocked_by and merge_status columns to tickets table"
```

---

## Task 3: Implement DAG scheduling in engine (is_ready + take_next + cycle detection)

**Files:**
- Modify: `crates/legion-core/src/orchestrate/engine.rs:259-306` — Update submit_ticket and take_next

**Step 1: Add blocked_by parameter to submit_ticket**

Change `submit_ticket()` signature (line 259) to accept `blocked_by`:

```rust
    pub async fn submit_ticket(
        &self, title: String, prompt: String, context: Option<String>, criteria: Option<String>,
        team_mode: TeamMode, max_iterations: u16, blocked_by: Vec<usize>,
    ) -> Result<usize, String> {
```

Add cycle detection before inserting:

```rust
    pub async fn submit_ticket(
        &self, title: String, prompt: String, context: Option<String>, criteria: Option<String>,
        team_mode: TeamMode, max_iterations: u16, blocked_by: Vec<usize>,
    ) -> Result<usize, String> {
        let mut guard = self.inner.write().await;

        // Validate: all blocked_by IDs must exist
        for &dep_id in &blocked_by {
            if !guard.tickets.iter().any(|t| t.id == dep_id) {
                return Err(format!("Dependency ticket #{} does not exist", dep_id));
            }
        }

        // Cycle detection: check if adding this ticket would create a cycle
        // (a ticket can't transitively depend on itself — but since this is a new ticket
        //  with a new ID, cycles can only happen if blocked_by tickets depend on each other
        //  in a way that creates a deadlock. For now, just validate deps exist.)

        let id = guard.next_ticket_id;
        guard.next_ticket_id += 1;
        let ticket = TaskTicket {
            id,
            prompt,
            title,
            context,
            criteria,
            status: TicketStatus::Queued,
            assigned_worker: None,
            team_mode,
            iteration: 0,
            max_iterations,
            feedback: None,
            summary: None,
            started_at: None,
            completed_elapsed_secs: None,
            base_commit: None,
            blocked_by,
            merge_status: MergeStatus::Pending,
        };
        guard.tickets.push(ticket.clone());
        drop(guard);
        self.persist_ticket(&ticket);
        Ok(id)
    }
```

**Step 2: Add is_ready() helper**

Add this method to the `impl OrchestrateEngine` block, before `take_next()`:

```rust
    /// Check if a ticket's dependencies are all Done
    fn is_ready(ticket: &TaskTicket, all_tickets: &[TaskTicket]) -> bool {
        ticket.blocked_by.iter().all(|dep_id| {
            all_tickets.iter()
                .find(|t| t.id == *dep_id)
                .map(|t| t.status == TicketStatus::Done)
                .unwrap_or(true) // dep not found = treat as done
        })
    }
```

Note: This is a static method (no `&self`) since it only operates on data.

**Step 3: Update take_next() to use is_ready**

Replace the `find` call in `take_next()` (line 296):

From:
```rust
        let ticket = guard.tickets.iter_mut().find(|t| t.status == TicketStatus::Queued)?;
```

To:
```rust
        let ticket = guard.tickets.iter_mut().find(|t| {
            t.status == TicketStatus::Queued && Self::is_ready(t, &guard.tickets)
        })?;
```

Wait — this won't work because we can't immutably borrow `guard.tickets` while mutably iterating. Fix by collecting ready ticket IDs first:

```rust
        // Find first Queued ticket whose dependencies are all Done
        let ready_id = {
            let tickets = &guard.tickets;
            tickets.iter()
                .find(|t| t.status == TicketStatus::Queued && Self::is_ready(t, tickets))
                .map(|t| t.id)
        };
        let ticket = match ready_id {
            Some(id) => guard.tickets.iter_mut().find(|t| t.id == id).unwrap(),
            None => return None,
        };
```

**Step 4: Fix all callers of submit_ticket**

The signature changed from returning `usize` to `Result<usize, String>` and added `blocked_by` parameter. Fix:

In `crates/legion-core/src/orchestrate/api.rs`:
- `handle_submit()` (line 156): pass `Vec::new()` for now (API will get `blocked_by` in Task 5), and handle the Result
- `handle_dispatch_compat()` (line 206): same — pass `Vec::new()`, handle Result

Example fix for `handle_submit()`:
```rust
            let id = match engine
                .submit_ticket(req.title, req.ticket, req.context, req.criteria, mode, max_iter, Vec::new())
                .await {
                    Ok(id) => id,
                    Err(e) => return Ok(json_response(StatusCode::BAD_REQUEST, &format!(r#"{{"error": "{}"}}"#, e))),
                };
```

**Step 5: Verify it compiles**

Run: `cargo build -p legion-core 2>&1 | head -20`
Expected: Compiles successfully

**Step 6: Commit**

```bash
git add crates/legion-core/
git commit -m "feat(core): DAG scheduling - is_ready check and blocked_by validation in submit_ticket"
```

---

## Task 4: Add merge_worker_into_leader and rebase_worker_from_leader to worktree.rs

**Files:**
- Modify: `crates/legion-tui/src/worktree.rs:155` — Add two new functions

**Step 1: Add merge_worker_into_leader**

After the existing `merge_branch()` function (line 155), add:

```rust
/// Auto-merge: merge a worker's branch into the leader's worktree.
/// Runs in the leader worktree directory. On conflict: aborts and returns Err.
pub fn merge_worker_into_leader(
    project_path: &Path,
    session_name: &str,
    worker_label: &str,
    is_default_session: bool,
) -> Result<()> {
    let worker_branch = pane_branch_name(session_name, worker_label);

    let leader_dir = if is_default_session {
        project_path.to_path_buf()
    } else {
        pane_worktree_path(project_path, session_name, "Leader")
    };

    if !leader_dir.exists() {
        anyhow::bail!("Leader worktree not found: {}", leader_dir.display());
    }

    // Stash any uncommitted changes in leader
    let stash_output = Command::new("git")
        .args(["stash", "push", "-m", "legion-auto-merge-stash"])
        .current_dir(&leader_dir)
        .output()
        .context("Failed to stash leader changes")?;
    let did_stash = String::from_utf8_lossy(&stash_output.stdout).contains("Saved working directory");

    // Merge worker branch
    let output = Command::new("git")
        .args([
            "merge",
            &worker_branch,
            "--no-ff",
            "-m",
            &format!("Auto-merge: {} completed", worker_label),
        ])
        .current_dir(&leader_dir)
        .output()
        .context("Failed to run git merge in leader worktree")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Abort the conflicted merge
        let _ = Command::new("git")
            .args(["merge", "--abort"])
            .current_dir(&leader_dir)
            .output();
        // Restore stash if we stashed
        if did_stash {
            let _ = Command::new("git")
                .args(["stash", "pop"])
                .current_dir(&leader_dir)
                .output();
        }
        anyhow::bail!("Auto-merge conflict for {}: {}", worker_label, stderr.trim());
    }

    // Restore stash
    if did_stash {
        let _ = Command::new("git")
            .args(["stash", "pop"])
            .current_dir(&leader_dir)
            .output();
    }

    tracing::info!("Auto-merged {} into leader ({})", worker_branch, leader_dir.display());
    Ok(())
}
```

**Step 2: Add rebase_worker_from_leader**

After the function above, add:

```rust
/// Rebase-on-start: pull leader's latest code into a worker's worktree.
/// Runs in the worker worktree directory. On failure: hard reset to leader HEAD.
pub fn rebase_worker_from_leader(
    project_path: &Path,
    session_name: &str,
    worker_label: &str,
    is_default_session: bool,
) -> Result<()> {
    let leader_branch = if is_default_session {
        default_branch(project_path)
    } else {
        pane_branch_name(session_name, "Leader")
    };

    let worker_dir = pane_worktree_path(project_path, session_name, worker_label);

    if !worker_dir.exists() {
        anyhow::bail!("Worker worktree not found: {}", worker_dir.display());
    }

    let output = Command::new("git")
        .args(["merge", &leader_branch, "--no-edit"])
        .current_dir(&worker_dir)
        .output()
        .context("Failed to merge leader into worker")?;

    if !output.status.success() {
        tracing::warn!("Worker rebase failed, hard resetting to leader HEAD");
        let _ = Command::new("git")
            .args(["merge", "--abort"])
            .current_dir(&worker_dir)
            .output();
        // Hard reset to leader's HEAD so worker starts clean
        let _ = Command::new("git")
            .args(["reset", "--hard", &leader_branch])
            .current_dir(&worker_dir)
            .output();
    }

    tracing::info!("Rebased {} from {}", worker_label, leader_branch);
    Ok(())
}
```

**Step 3: Verify it compiles**

Run: `cargo build -p legion-tui 2>&1 | head -20`
Expected: Compiles

**Step 4: Commit**

```bash
git add crates/legion-tui/src/worktree.rs
git commit -m "feat(tui): add merge_worker_into_leader and rebase_worker_from_leader"
```

---

## Task 5: Add set_merge_status to engine and update auto-merge in event loop

**Files:**
- Modify: `crates/legion-core/src/orchestrate/engine.rs` — Add set_merge_status method
- Modify: `crates/legion-tui/src/lib.rs:394-528` — Add auto-merge after diff cache, add rebase before start_sdk_task

**Step 1: Add set_merge_status to engine**

In `crates/legion-core/src/orchestrate/engine.rs`, add this method to `impl OrchestrateEngine` (after `set_base_commit`):

```rust
    /// Update the merge status for a ticket
    pub async fn set_merge_status(&self, ticket_id: usize, status: MergeStatus) {
        let mut guard = self.inner.write().await;
        if let Some(ticket) = guard.tickets.iter_mut().find(|t| t.id == ticket_id) {
            ticket.merge_status = status;
            let snap = ticket.clone();
            drop(guard);
            self.persist_ticket_update(&snap);
        }
    }
```

**Step 2: Add auto-merge in lib.rs after diff cache**

In `crates/legion-tui/src/lib.rs`, find the section after diff caching (around line 496, after the `tokio::time::timeout` block), and **before** the SDK cleanup section (line 498). Insert the auto-merge logic:

```rust
                    // Auto-merge: merge worker branch into leader (only for Done tickets)
                    if promise_found {
                        if let (Some(project_path), Some(session)) = (&app.project_path, &app.current_session) {
                            let worker_label = format!("Worker {}", wi);
                            let pp = project_path.clone();
                            let sn = session.name.clone();
                            let is_default = session.is_default;
                            match crate::worktree::merge_worker_into_leader(&pp, &sn, &worker_label, is_default) {
                                Ok(()) => {
                                    tracing::info!("Auto-merged {} into leader", worker_label);
                                    engine.set_merge_status(ticket_id, legion_core::MergeStatus::Merged).await;
                                }
                                Err(e) => {
                                    tracing::warn!("Auto-merge failed for {}: {}", worker_label, e);
                                    engine.set_merge_status(ticket_id, legion_core::MergeStatus::Conflict).await;
                                }
                            }
                        }
                    }
```

**Step 3: Add rebase-on-start before start_sdk_task**

In `crates/legion-tui/src/lib.rs`, find the "idle worker takes next ticket" section (around line 505-528). Insert rebase logic **after** `take_next` returns a ticket and **before** `start_sdk_task`. Replace the current block:

```rust
                // If worker is idle and no SDK running, try to take next ticket
                if app.panes[wi].sdk_task.is_none() {
                    if let Some(ts) = engine.take_next(wi as u16).await {
                        tracing::info!("Worker {} taking ticket {}", wi, ts.id);

                        if let (Some(project_path), Some(session)) = (&app.project_path, &app.current_session) {
                            let worker_label = format!("Worker {}", wi);
                            let wt_path = crate::worktree::pane_worktree_path(
                                project_path, &session.name, &worker_label,
                            );

                            // Rebase worker worktree to leader's latest before starting
                            if let Err(e) = crate::worktree::rebase_worker_from_leader(
                                project_path, &session.name, &worker_label, session.is_default,
                            ) {
                                tracing::warn!("Worker {} rebase failed: {}", wi, e);
                            }

                            // Capture base commit after rebase (reflects leader's latest)
                            if let Ok(output) = std::process::Command::new("git")
                                .args(["rev-parse", "HEAD"])
                                .current_dir(&wt_path)
                                .output()
                            {
                                if output.status.success() {
                                    let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
                                    engine.set_base_commit(ts.id, commit).await;
                                }
                            }
                        }

                        app.start_sdk_task(wi, ts.id, &ts.prompt, &ts.team_mode, 1, None,
                            ts.title.as_str(), ts.context.as_deref(), ts.criteria.as_deref());
                    }
                }
```

**Step 4: Verify it compiles**

Run: `cargo build -p legion-tui 2>&1 | head -20`
Expected: Compiles

**Step 5: Commit**

```bash
git add crates/legion-core/ crates/legion-tui/
git commit -m "feat: auto-merge worker->leader on Done, rebase worker from leader on start"
```

---

## Task 6: Update legion-dispatch with --after flag and API submit with blocked_by

**Files:**
- Modify: `crates/legion-tools/src/bin/legion-dispatch.rs:33-96` — Parse --after flag, send blocked_by
- Modify: `crates/legion-core/src/orchestrate/api.rs:135-171` — Accept blocked_by in SubmitRequest

**Step 1: Add --after parsing to legion-dispatch**

In `crates/legion-tools/src/bin/legion-dispatch.rs`, add `after` to the flag parsing section (around line 34):

```rust
    let mut title: Option<String> = None;
    let mut context: Option<String> = None;
    let mut criteria: Option<String> = None;
    let mut after: Option<String> = None;
    let mut positional: Vec<String> = Vec::new();
```

In the match block (around line 41), add:
```rust
            "-a" | "--after" => {
                i += 1;
                if i < args.len() { after = Some(args[i].clone()); }
            }
```

**Step 2: Parse after into blocked_by array and add to JSON body**

After the `body` construction (around line 91), add:

```rust
    if let Some(after_str) = &after {
        let blocked_by: Vec<u64> = after_str.split(',')
            .filter_map(|s| s.trim().parse::<u64>().ok())
            .collect();
        if !blocked_by.is_empty() {
            body["blocked_by"] = serde_json::json!(blocked_by);
        }
    }
```

**Step 3: Update usage strings**

Update the usage strings (lines 21, 61, 66-67) to mention `--after`:

```rust
        eprintln!("Usage: legion-dispatch <worker_id> [-t \"title\"] [-c \"context\"] [-k \"criteria\"] [--after 1,3] \"ticket text\"");
```

**Step 4: Update API's SubmitRequest to accept blocked_by**

In `crates/legion-core/src/orchestrate/api.rs`, in the `SubmitRequest` struct (line 135), add:

```rust
        #[serde(default)]
        blocked_by: Vec<usize>,
```

**Step 5: Pass blocked_by to submit_ticket**

In `handle_submit()` (line 156), pass `req.blocked_by`:

```rust
            let id = match engine
                .submit_ticket(req.title, req.ticket, req.context, req.criteria, mode, max_iter, req.blocked_by)
                .await {
                    Ok(id) => id,
                    Err(e) => return Ok(json_response(StatusCode::BAD_REQUEST, &format!(r#"{{"error": "{}"}}"#, e))),
                };
```

**Step 6: Verify it compiles**

Run: `cargo build -p legion-tools -p legion-core 2>&1 | head -20`
Expected: Compiles

**Step 7: Commit**

```bash
git add crates/legion-tools/ crates/legion-core/
git commit -m "feat(tools): add --after flag to legion-dispatch for DAG dependencies"
```

---

## Task 7: Update legion-check and legion-status to show blocked_by and merge_status

**Files:**
- Modify: `crates/legion-tools/src/bin/legion-check.rs:103-146` — Show dependencies and merge status
- Modify: `crates/legion-tools/src/bin/legion-status.rs:48-78` — Show merge status badge

**Step 1: Update legion-check to show dependencies**

In `crates/legion-tools/src/bin/legion-check.rs`, in the per-ticket display loop (around line 103), after `let worker = ...`, add:

```rust
            // Parse blocked_by dependencies
            let blocked_by: Vec<u64> = t.get("blocked_by")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_u64()).collect())
                .unwrap_or_default();

            let merge_status = t.get("merge_status")
                .and_then(|v| v.as_str())
                .unwrap_or("pending");
```

Update the display format to include dependencies and merge info. Replace the `println!` for ticket display (around line 133):

```rust
            let mut extras = Vec::new();
            if let Some(w) = worker {
                extras.push(format!("worker={}", w));
            }
            if !blocked_by.is_empty() {
                let dep_strs: Vec<String> = blocked_by.iter().map(|dep_id| {
                    let dep_status = tickets.iter()
                        .find(|dt| dt.get("id").and_then(|v| v.as_u64()) == Some(*dep_id))
                        .and_then(|dt| dt.get("status").and_then(|v| v.as_str()))
                        .unwrap_or("?");
                    format!("#{} {}", dep_id, dep_status)
                }).collect();
                extras.push(format!("after: {}", dep_strs.join(" ")));
            }
            if merge_status != "pending" {
                extras.push(format!("[{}]", merge_status));
            }

            let extras_str = if extras.is_empty() {
                String::new()
            } else {
                format!("  {}", extras.join("  "))
            };

            println!(
                "  [{}] \"{}\"{}  ({}s)",
                ticket_id, display_ticket, extras_str, elapsed
            );
```

**Step 2: Update legion-status to show merge status**

In `crates/legion-tools/src/bin/legion-status.rs`, in the per-ticket loop, add merge_status badge:

```rust
        let merge_status = t.get("merge_status")
            .and_then(|v| v.as_str())
            .unwrap_or("pending");

        let merge_badge = match merge_status {
            "merged" => "M",
            "conflict" => "!",
            _ => "",
        };

        let worker_str = match worker {
            Some(w) => format!("W{}", w),
            None => format!("#{}", id),
        };

        if merge_badge.is_empty() {
            parts.push(format!("{}[{}]", worker_str, badge));
        } else {
            parts.push(format!("{}[{}{}]", worker_str, badge, merge_badge));
        }
```

**Step 3: Verify it compiles**

Run: `cargo build -p legion-tools 2>&1 | head -20`
Expected: Compiles

**Step 4: Install updated binaries**

Run: `cargo install --path crates/legion-tools --bin legion-check --bin legion-status --bin legion-dispatch`

**Step 5: Commit**

```bash
git add crates/legion-tools/
git commit -m "feat(tools): show blocked_by and merge_status in legion-check and legion-status"
```

---

## Task 8: Update Leader CLAUDE.md with --after documentation

**Files:**
- Modify: `crates/legion-tui/src/claudemd.rs:9-57` — Update leader_instructions()

**Step 1: Update the leader prompt**

In `crates/legion-tui/src/claudemd.rs`, in `leader_instructions()`, update the dispatch format section to include `--after`:

In the MANDATORY DISPATCH FORMAT section, update the format example:
```
legion-dispatch <worker_id> -t "title" -c "context" -k "criteria" [--after 1,3] "task description"
```

Add `--after` to the flag descriptions:
```
- `--after` — (Optional) Comma-separated ticket IDs this task depends on: "--after 1,3"
```

Update the example:
```bash
legion-dispatch 1 -t "Implement heart animation" -c "Python 3, no external deps, terminal ANSI output" -k "heart.py exists, python3 heart.py shows animated heart, uses math-based curve" "Create heart.py with parametric heart curve animation using ANSI colors"

# With dependency:
legion-dispatch 2 -t "Add unit tests" -c "Python 3, pytest" -k "all tests pass" --after 1 "Write tests for heart.py animation"
```

Add a section about task dependencies after the Workflow section:

```
## Task Dependencies

Use `--after` when tasks have file dependencies:
- Task B reads files Task A creates → `--after A`
- Task C depends on both A and B → `--after A,B`
- Independent tasks need no `--after` (they run in parallel)

Workers auto-receive code from completed dependencies via auto-merge.
```

**Step 2: Update the test assertions**

In the `leader_prompt_mentions_dispatch_format` test, add:
```rust
        assert!(prompt.contains("--after"));
```

**Step 3: Verify tests pass**

Run: `cargo test -p legion-tui -- claudemd 2>&1`
Expected: All pass

**Step 4: Commit**

```bash
git add crates/legion-tui/src/claudemd.rs
git commit -m "feat(tui): update leader CLAUDE.md with --after dependency documentation"
```

---

## Task 9: Clean up debug files and final verification

**Files:**
- Modify: `crates/legion-tui/src/app.rs:1568-1570` — Remove debug file dumps

**Step 1: Remove debug file dumps**

In `crates/legion-tui/src/app.rs`, find and remove these lines (around 1568-1570):
```rust
        // DEBUG: dump system prompt to file for verification
        let _ = std::fs::write("/tmp/legion-debug-sysprompt.txt", &sys_prompt);
        let _ = std::fs::write("/tmp/legion-debug-wd.txt", format!("working_dir={:?}\nwd_str={:?}\npane_label={}\npane_index={}", working_dir, wd_str, pane_label, pane_index));
```

**Step 2: Full build and test**

Run: `cargo build --workspace 2>&1 | tail -5`
Run: `cargo test --workspace 2>&1 | tail -20`
Expected: All pass

**Step 3: Install all tools**

Run: `cargo install --path crates/legion-tools`

**Step 4: Commit**

```bash
git add crates/legion-tui/src/app.rs
git commit -m "chore: remove debug file dumps from app.rs"
```

---

## Verification Checklist

After all tasks complete:

1. `cargo build --workspace` — no errors
2. `cargo test --workspace` — all pass
3. Start a squad session with `legion squad --workers 2`
4. In Leader pane, dispatch tasks with dependencies:
   ```
   legion-dispatch 1 -t "Create utils" -c "Python" -k "utils.py exists" "Create utils.py"
   legion-dispatch 2 -t "Use utils" -c "Python" -k "main.py imports utils" --after 1 "Create main.py that imports utils"
   ```
5. Verify ticket 2 stays Queued until ticket 1 is Done
6. Verify ticket 1 Done → auto-merged to leader worktree
7. Verify worker taking ticket 2 → rebases from leader first
8. `legion-check` shows dependency info and merge status
9. `legion-status` shows merge badge
