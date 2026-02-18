# Branch-Session Binding Design

## Problem

When starting Legion without a default session, the user sees a blank text input asking for a session name. This provides no guidance. Sessions should be automatically associated with the current git branch. Additionally, when a user completes work outside Legion (merges code, deletes branch), the session list should clearly reflect this state.

## Design

### DB Schema Changes

Add two columns to `squad_sessions`:

```sql
ALTER TABLE squad_sessions ADD COLUMN base_branch TEXT;
ALTER TABLE squad_sessions ADD COLUMN base_commit TEXT;
```

- `base_branch`: branch name at session creation (e.g. `feature/auth`)
- `base_commit`: commit SHA at session creation (e.g. `a1b2c3d...`)
- Both nullable for backward compatibility with existing sessions

`SquadSession` struct adds:
```rust
pub base_branch: Option<String>,
pub base_commit: Option<String>,
```

### First-Launch Guided Flow

When no default session exists for the current project:

1. Detect current branch: `git branch --show-current`
2. Detect current commit: `git rev-parse HEAD`
3. Sanitize branch name for session name (e.g. `feature/auth` → `feature-auth`)
4. Show confirmation dialog:
   ```
   Create session 'feature-auth' (branch: feature/auth, 3 workers)? [Enter/Esc]
   ```
5. Enter → create default session with `base_branch` and `base_commit` populated
6. Esc → show full session input form (current behavior)

### Session List Display

Branch status is checked when the session list is opened via `git branch --list <base_branch>`.

| State | Display |
|-------|---------|
| Branch exists | `◉ feature-auth (3w) [default] ← feature/auth` |
| Branch deleted | `○ feature-auth (3w) [default] ⚠ branch 'feature/auth' deleted (a1b2c3d)` |
| Legacy session (no branch info) | `○ old-session (3w)` (unchanged) |

### Branch-Deleted Recovery Flow

When user tries to resume a session whose `base_branch` no longer exists:

```
⚠ Branch 'feature/auth' has been deleted (base commit: a1b2c3d)

  [1] Bind to current branch (main) and continue
  [2] Select another branch
  [3] Create new branch from base commit (a1b2c3d)
  [4] Cancel
```

- Option 1: Update `base_branch` to current branch, `base_commit` to current HEAD
- Option 2: List local branches for selection, then update bindings
- Option 3: Run `git branch <name> <base_commit>`, bind to new branch
- Option 4: Return to session list

### Runtime Branch Change Detection

In the event loop, periodically (every 5s) check `git branch --show-current`:

- If different from session's `base_branch`, show prompt:
  ```
  Branch changed: feature/auth → main
    [1] Switch session to 'main' (rebuild worktrees)
    [2] Switch session to 'main' (rebase worktrees)
    [3] Ignore
  ```

### Manual Branch Switching

Add "Switch Branch" option to the main menu (Ctrl+P):

- Lists local branches
- On selection, prompts rebuild/rebase worktree choice
- Updates `base_branch` and `base_commit` in DB

## Files Affected

- `crates/legion-db/src/schema.rs` — migration for new columns
- `crates/legion-db/src/repo.rs` — SquadSession struct, query methods
- `crates/legion-tui/src/app.rs` — session creation, branch detection helpers
- `crates/legion-tui/src/lib.rs` — first-launch flow, branch change detection in event loop
- `crates/legion-tui/src/input.rs` — recovery dialog, branch switch menu
- `crates/legion-tui/src/ui.rs` — session list rendering with branch status
