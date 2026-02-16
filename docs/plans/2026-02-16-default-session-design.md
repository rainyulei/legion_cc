# Default Session & Session Lifecycle Redesign

## Problem

Squad mode currently requires manual session selection on every startup. There is no persistent "home base" session, no way to delete sessions from the list, and no way to migrate task records when completing a session.

## Design

### 1. Default Session

A locked default session that always exists and serves as the merge target for all other sessions.

**Creation:**
- First squad startup: prompt user for default session name (pre-filled with git default branch name, e.g. `main`)
- Stored in DB with `is_default = true`
- Subsequent startups: auto-resume default session, skip session list popup

**Worktree behavior:**
- Leader works directly in the main repository (no worktree created)
- Workers still get independent worktrees as usual

**Protection:**
- Cannot be deleted
- Cannot be completed

### 2. Session Lifecycle

```
Create -> active -> Complete (merge + handle records + delete session)
                    or Delete (remove everything)
```

No `completed` status during active use. After completion, session moves to a read-only completed list in the DB.

### 3. Complete Session

Three-phase process:

**Phase 1: Code merge**
- Merge all worker branches to git default branch
- Clean up worktrees and git branches

**Phase 2: Record handling (user choice)**
- **Delete**: Remove all tickets + ticket_logs for this session
- **Migrate to default session**: Update `session_name` on tickets and ticket_logs to default session name, set `origin_session` field to original session name

**Phase 3: Cleanup**
- Mark session as `status = 'completed'` in DB (preserved for history)
- Session appears in "Completed" section of session list (read-only)

### 4. Delete Session

Direct removal of everything:

1. Git worktrees -> `git worktree remove --force`
2. Git branches -> `git branch -D legion/<session>/*`
3. DB: `DELETE FROM tickets WHERE session_name = ?`
4. DB: `DELETE FROM ticket_logs WHERE session_name = ?`
5. DB: `DELETE FROM pane_configs` for session-related labels
6. DB: `DELETE FROM squad_sessions WHERE name = ?`
7. Filesystem: remove `<project>-legion/<session>/` directory

**Confirmation popup shows:**
- Session name
- Number of pending tasks (if any, with warning)
- List of resources to be deleted (worktrees, branches, tickets, logs, configs)

Default session: `d` key is disabled.

### 5. Session List Popup

Enhanced with keyboard shortcuts and two sections:

```
+-- Sessions -----------------------------------+
|                                               |
|  Active:                                      |
|  > [default] main           3 workers         |
|    feature-auth             2 workers         |
|                                               |
|  Completed:                                   |
|    feature-login    Feb 15  merged            |
|    bugfix-cart      Feb 12  merged            |
|                                               |
|  [Enter=Resume] [n=New] [d=Delete]            |
|  [c=Complete] [x=Remove from list]            |
+-----------------------------------------------+
```

**Keys:**
- `Enter` = Resume selected active session
- `n` = New session
- `d` = Delete session (disabled for default, shows confirm popup)
- `c` = Complete session (disabled for default, shows complete flow)
- `x` = Remove completed session from history list
- `Esc` = Back

**Display:**
- Active sessions on top, completed below (grayed out)
- Default session always first with `[default]` tag
- Completed sessions show date and status, not interactive (except `x` to remove)

### 6. DB Schema Changes

**squad_sessions table:**
```sql
ALTER TABLE squad_sessions ADD COLUMN is_default INTEGER NOT NULL DEFAULT 0;
-- completed_at column kept for completed session display
```

**tickets table:**
```sql
ALTER TABLE tickets ADD COLUMN origin_session TEXT;
```

### 7. New DB Methods

```rust
// Get the default session for current project
fn get_default_session(project_path: &str) -> Result<Option<SquadSession>>

// Delete all data associated with a session
fn delete_session_all_data(session_name: &str) -> Result<()>

// Migrate tickets + logs from one session to another, setting origin_session
fn migrate_tickets_to_session(from: &str, to: &str) -> Result<()>

// Count pending/in_progress tickets for a session
fn count_pending_tickets(session_name: &str) -> Result<usize>

// Permanently remove a completed session record
fn remove_completed_session(name: &str) -> Result<()>
```

### 8. Files to Modify

| File | Changes |
|------|---------|
| `legion-db/src/schema.rs` | Add `is_default` column to squad_sessions, `origin_session` to tickets |
| `legion-db/src/repo.rs` | Add `is_default` to SquadSession struct, new methods above |
| `legion-tui/src/app.rs` | `create_session` accepts is_default, default Leader skips worktree, complete_session does merge+record handling+cleanup, delete_session removes everything |
| `legion-tui/src/lib.rs` | Startup: check default session first, auto-resume if exists, else show first-time setup |
| `legion-tui/src/input.rs` | SessionList: add `d`/`c`/`n`/`x` keys, delete confirm popup, complete session flow with record choice |
| `legion-tui/src/ui.rs` | Render delete confirm popup, complete session record choice popup, session list with Active/Completed sections and `[default]` tag |
| `legion-tui/src/worktree.rs` | `create_session_worktrees` accepts is_default flag to skip Leader worktree |

### 9. Startup Flow

```
run_squad()
  |
  v
Check DB for default session (is_default=true, matching project_path)
  |
  +-- Found & active --> auto resume default session --> Normal mode
  |
  +-- Not found --> Show "First Time Setup" popup
                     Pre-fill name with git default branch
                     User confirms --> create default session (is_default=true)
                     --> Normal mode
```

If user wants to switch sessions, use Ctrl+P > Switch Session to open session list.
