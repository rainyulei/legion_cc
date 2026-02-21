# Teams/Roles Leader-Worker Integration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fully integrate Teams and Roles into Leader and Worker prompt generation — dynamic team directory in Leader, complete collaboration framework in Worker, and user-editable team_prompt field in TUI.

**Architecture:** Add `team_prompt` column to teams DB table, update Team struct and all CRUD paths, rewrite `leader_instructions()` and `worker_instructions()` in claudemd.rs, update TUI TeamForm to support the new field.

**Tech Stack:** Rust, SQLite (rusqlite), ratatui TUI, legion-db/legion-tui crates

---

### Task 1: DB Schema — Add team_prompt Column

**Files:**
- Modify: `crates/legion-db/src/schema.rs:122-181`
- Modify: `crates/legion-db/src/repo.rs:93-100` (Team struct)

**Step 1: Add migration in `init_db()`**

In `crates/legion-db/src/schema.rs`, after line 133 (the `merge_status` migration), add:

```rust
let _ = conn.execute("ALTER TABLE teams ADD COLUMN team_prompt TEXT DEFAULT ''", []);
```

**Step 2: Update seed data to include team_prompt**

Replace the 5 existing seed team INSERT statements (lines 166-179) with versions that include team_prompt. Since we use `INSERT OR IGNORE`, existing rows won't be updated, so we also need UPDATE statements for the team_prompt column:

```rust
// After the existing INSERT OR IGNORE statements for teams, add team_prompt updates:
let _ = conn.execute("UPDATE teams SET team_prompt = ?1 WHERE id = 'tech_lead_team' AND team_prompt = ''",
    ["This team follows a structured development workflow: 1) Tech Lead analyzes requirements and designs architecture 2) Engineer implements with strict TDD 3) QA validates all acceptance criteria. Communication: Sequential delegation by default. Use parallel only for independent review tasks."]);

let _ = conn.execute("UPDATE teams SET team_prompt = ?1 WHERE id = 'fullstack_team' AND team_prompt = ''",
    ["Architecture-driven team: 1) Architect designs system structure and evaluates trade-offs 2) Engineer implements following the architecture 3) QA tests comprehensively. Communication: Architect reviews before Engineer starts. Parallel QA alongside implementation for independent test writing."]);

let _ = conn.execute("UPDATE teams SET team_prompt = ?1 WHERE id = 'backend_team' AND team_prompt = ''",
    ["Focused backend pair: 1) Tech Lead plans and reviews 2) Engineer implements with TDD. Communication: Sequential — plan first, then implement."]);

let _ = conn.execute("UPDATE teams SET team_prompt = ?1 WHERE id = 'qa_team' AND team_prompt = ''",
    ["Quality-focused pair: 1) Engineer implements features 2) QA writes independent tests and validates. Communication: Parallel — QA can write test specs while Engineer implements."]);
```

(Solo team gets no team_prompt — empty string is fine.)

**Step 3: Add `team_prompt` to Team struct**

In `crates/legion-db/src/repo.rs`, modify the `Team` struct (line 93):

```rust
pub struct Team {
    pub id: String,
    pub name: String,
    pub description: String,
    pub role_ids: Vec<String>,
    pub is_builtin: bool,
    pub created_at: i64,
    pub team_prompt: String,  // NEW
}
```

**Step 4: Build to find all compilation errors**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo build 2>&1`

This will show every place that constructs a `Team` — you'll need to add `team_prompt: String::new()` or read from DB in each location. Fix all compilation errors.

**Step 5: Update all Team query/insert methods in repo.rs**

Update these methods to include `team_prompt`:

- `list_teams()` (line 762): Add `team_prompt` to SELECT and Team construction
- `get_team()` (line 781): Same
- `upsert_team()` (line 802): Add `team_prompt` to INSERT and params

For `list_teams()` and `get_team()`, the SELECT becomes:
```sql
SELECT id, name, description, role_ids, is_builtin, created_at, COALESCE(team_prompt, '') as team_prompt FROM teams ...
```

Use `COALESCE` to handle rows created before the migration that might have NULL.

For `upsert_team()`, the INSERT becomes:
```sql
INSERT OR REPLACE INTO teams (id, name, description, role_ids, is_builtin, created_at, team_prompt) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
```

**Step 6: Update DB tests**

In `crates/legion-db/src/repo.rs` tests, update any test that constructs a `Team` to include `team_prompt: String::new()` or a test value. Key tests: `crud_custom_team`, `seed_teams_exist`, `get_team_roles_resolves`.

Add a new test:

```rust
#[test]
fn team_prompt_persists() {
    let repo = test_repo();
    let team = Team {
        id: "test_prompt".into(),
        name: "Test".into(),
        description: "Desc".into(),
        role_ids: vec![],
        is_builtin: false,
        created_at: 100,
        team_prompt: "Custom collaboration guide".into(),
    };
    repo.upsert_team(&team).unwrap();
    let loaded = repo.get_team("test_prompt").unwrap().unwrap();
    assert_eq!(loaded.team_prompt, "Custom collaboration guide");
}
```

**Step 7: Verify**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo build && cargo test`
Expected: All tests pass, no warnings about team_prompt.

---

### Task 2: Leader Prompt — Dynamic Team Directory

**Files:**
- Modify: `crates/legion-tui/src/claudemd.rs:9-104` (leader_instructions)
- Modify: `crates/legion-tui/src/claudemd.rs:329-344` (write_squad_claude_md)
- Modify: `crates/legion-tui/src/claudemd.rs:366-376` (test)
- Modify: `crates/legion-tui/src/app.rs:1089` (call site)
- Modify: `crates/legion-tui/src/app.rs:1224` (call site)

**Step 1: Change leader_instructions() signature**

In `crates/legion-tui/src/claudemd.rs`, change:

```rust
pub fn leader_instructions(worker_count: u16) -> String {
```

to:

```rust
pub fn leader_instructions(worker_count: u16, teams: &[(String, String, Vec<String>)]) -> String {
```

Where each tuple is `(team_id, team_name, role_names)`.

**Step 2: Replace hardcoded team list with dynamic directory**

In the format string of `leader_instructions()`, replace line 27:
```
- `--team` — (Optional) Team template: tech_lead_team (default), fullstack_team, backend_team, qa_team, solo
```

With a dynamically generated section. After the format string, build the team directory:

```rust
pub fn leader_instructions(worker_count: u16, teams: &[(String, String, Vec<String>)]) -> String {
    let mut team_table = String::from("| Team ID | Name | Roles |\n|---------|------|-------|\n");
    for (id, name, roles) in teams {
        let roles_str = if roles.is_empty() {
            "(no roles, TDD mode)".to_string()
        } else {
            roles.join(", ")
        };
        team_table.push_str(&format!("| {} | {} | {} |\n", id, name, roles_str));
    }

    format!(
        r#"# Squad Leader

You coordinate a team of {} autonomous Workers.

## MANDATORY DISPATCH FORMAT — READ THIS FIRST

Every `legion-dispatch` call MUST include ALL four parts. The command WILL FAIL without -t, -c, and -k:

```
legion-dispatch <worker_id> [--team <team_name>] -t "title" -c "context" -k "criteria" [--after N,M] "task description"
```

- `-t` — Short title (3-6 words): "Implement heart animation"
- `-c` — Context (language, dependencies, constraints): "Python 3, no external deps, terminal ANSI output"
- `-k` — Success criteria (testable conditions): "heart.py exists, python3 heart.py runs, uses math curve"
- `--after` — (Optional) Comma-separated ticket IDs this task depends on: "--after 1,3"
- `--team` — (Optional) Team ID from the Available Teams below. Default: tech_lead_team
- Last arg — Full task description with all implementation details

**IMPORTANT:** Do NOT include working directory paths in `-c`. Each Worker has its own dedicated worktree — they will create files in their current directory automatically.

## Available Teams

{}
Use `--team <team_id>` to assign a team to a ticket. Default: tech_lead_team

... (rest of prompt unchanged from line 32 onward: Example, With dependency, With team, Workflow, etc.)
"#,
        worker_count,
        team_table,
    )
}
```

Keep the rest of the prompt (Workflow, Task Dependencies, Tools, 任务分解提示, Commands, Rules) exactly as-is. Just remove the hardcoded "With team" example that references specific team names and replace with a generic one using `<team_id>`.

**Step 3: Update write_squad_claude_md()**

This function at line 329 currently calls `leader_instructions(worker_count)`. Change to pass an empty teams slice (this function doesn't have DB access):

```rust
pub fn write_squad_claude_md(worker_count: u16) -> Result<(PathBuf, Vec<PathBuf>)> {
    // ... existing code ...
    let leader_path = dir.join("leader-CLAUDE.md");
    fs::write(&leader_path, leader_instructions(worker_count, &[]))?;
    // ... rest unchanged
}
```

**Step 4: Update call site in app.rs — init_squad_session (line 1089)**

Add a helper method to App that loads teams with their role names:

```rust
fn load_teams_for_leader(&self) -> Vec<(String, String, Vec<String>)> {
    if let Some(ref engine) = self.orchestrate {
        if let Some(db) = engine.db() {
            if let Ok(db_lock) = db.lock() {
                if let Ok(teams) = db_lock.list_teams() {
                    return teams.into_iter().map(|t| {
                        let role_names: Vec<String> = t.role_ids.iter().filter_map(|rid| {
                            db_lock.get_role(rid).ok().flatten().map(|r| r.name)
                        }).collect();
                        (t.id, t.name, role_names)
                    }).collect();
                }
            }
        }
    }
    Vec::new()
}
```

Then at line 1089:
```rust
let teams_for_leader = self.load_teams_for_leader();
let leader_prompt = crate::claudemd::leader_instructions(worker_count, &teams_for_leader);
```

**Step 5: Update call site in app.rs — add_worker_panes (line 1224)**

```rust
let prompt = if i == 0 {
    let teams_for_leader = self.load_teams_for_leader();
    crate::claudemd::leader_instructions(worker_count, &teams_for_leader)
} else {
    // ... worker prompt unchanged
};
```

**Step 6: Update test `leader_prompt_mentions_dispatch_format`**

```rust
#[test]
fn leader_prompt_mentions_dispatch_format() {
    let teams = vec![
        ("tech_lead_team".to_string(), "Tech Lead Team".to_string(), vec!["Tech Lead".to_string(), "Engineer".to_string()]),
        ("solo".to_string(), "Solo".to_string(), vec![]),
    ];
    let prompt = leader_instructions(2, &teams);
    assert!(prompt.contains("MANDATORY DISPATCH FORMAT"));
    assert!(prompt.contains("legion-dispatch"));
    assert!(prompt.contains("legion-check"));
    assert!(prompt.contains("--after"));
    assert!(prompt.contains("/split-tickets"));
    assert!(prompt.contains("任务分解提示"));
    assert!(prompt.contains("不要直接开始执行"));
    // New: dynamic team table
    assert!(prompt.contains("Available Teams"));
    assert!(prompt.contains("tech_lead_team"));
    assert!(prompt.contains("Tech Lead, Engineer"));
    assert!(prompt.contains("solo"));
}
```

**Step 7: Verify**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo build && cargo test`

---

### Task 3: Worker Prompt — Complete Collaboration Framework

**Files:**
- Modify: `crates/legion-tui/src/claudemd.rs:112-218` (worker_instructions)
- Modify: `crates/legion-tui/src/claudemd.rs:412-484` (tests)
- Modify: `crates/legion-tui/src/app.rs:1627-1693` (resolve + call site)

**Step 1: Change worker_instructions() signature**

```rust
pub fn worker_instructions(
    worker_id: u16,
    working_dir: Option<&str>,
    team_roles: Option<&[(String, String, String)]>,
    team_prompt: Option<&str>,  // NEW
) -> String {
```

**Step 2: Rewrite Agent Team section**

Replace the current Agent Team generation (lines 134-160) with the full collaboration framework:

```rust
if let Some(roles) = team_roles {
    if !roles.is_empty() {
        prompt.push_str("\n## Agent Team\n\n");

        // Team objective from team_prompt
        if let Some(tp) = team_prompt {
            if !tp.is_empty() {
                prompt.push_str("### Team Objective\n\n");
                prompt.push_str(tp);
                prompt.push_str("\n\n");
            }
        }

        prompt.push_str("### Your Teammates\n\n");
        for (role_id, role_name, prompt_template) in roles {
            prompt.push_str(&format!(
                "#### {} (ID: {})\n{}\n\n",
                role_name, role_id, prompt_template
            ));
        }

        prompt.push_str(
            r#"### Communication Protocol

You have two delegation modes. Choose based on task structure:

**Sequential (default):** Delegate to one teammate at a time via the Task tool. Wait for their result, review it, then decide the next step.
- Use when: tasks have dependencies, quality review needed between steps
- Example flow: Tech Lead designs → Engineer implements → QA validates

**Parallel (broadcast):** Delegate to multiple teammates simultaneously via multiple Task tool calls in one message.
- Use when: tasks are independent, need diverse perspectives, or time-sensitive
- Example: Multiple engineers implement different components; QA writes test specs while Engineer codes

### Entropy Reduction Rules

1. Each role focuses ONLY on their stated domain — do not cross-assign responsibilities
2. When reviewing teammate output, provide SPECIFIC actionable feedback (not "looks good")
3. If roles produce conflicting results, YOU (team lead) make the final decision based on success criteria
4. Do not re-delegate the same work without concrete changes to the instructions
5. Prefer fewer, more targeted delegations over many vague ones

### Completion
When ALL success criteria are met, output `<promise>DONE</promise>` with a summary of what each role contributed.

"#
        );
    }
}
```

**Step 3: Update all call sites that call worker_instructions()**

Search for all calls to `worker_instructions` and add the `team_prompt` parameter:

1. **claudemd.rs line 339** (`write_squad_claude_md`):
   ```rust
   fs::write(&path, worker_instructions(i, None, None, None))?;
   ```

2. **app.rs line 1678** (Solo mode):
   ```rust
   crate::claudemd::worker_instructions(pane_index as u16, wd_str.as_deref(), Some(&[]), None)
   ```

3. **app.rs line 1688** (no roles found fallback):
   ```rust
   crate::claudemd::worker_instructions(pane_index as u16, wd_str.as_deref(), None, None)
   ```

4. **app.rs line 1690** (team with roles):
   ```rust
   // Need to also fetch team_prompt here
   let team_prompt_str = self.resolve_team_prompt(team_name);
   crate::claudemd::worker_instructions(
       pane_index as u16,
       wd_str.as_deref(),
       Some(&team_roles),
       team_prompt_str.as_deref(),
   )
   ```

5. **app.rs line 1227** (add_worker_panes worker branch):
   ```rust
   crate::claudemd::worker_instructions(i as u16, wd_str.as_deref(), None, None)
   ```

**Step 4: Add resolve_team_prompt() helper to App**

In `crates/legion-tui/src/app.rs`, near `resolve_team_roles()` (line 1627):

```rust
fn resolve_team_prompt(&self, team_id: &str) -> Option<String> {
    if let Some(ref engine) = self.orchestrate {
        if let Some(db) = engine.db() {
            if let Ok(db_lock) = db.lock() {
                if let Ok(Some(team)) = db_lock.get_team(team_id) {
                    if !team.team_prompt.is_empty() {
                        return Some(team.team_prompt);
                    }
                }
            }
        }
    }
    None
}
```

**Step 5: Update tests**

Update `worker_prompt_with_team_roles` test:
```rust
#[test]
fn worker_prompt_with_team_roles() {
    let roles = vec![
        ("tech_lead".to_string(), "Technical Lead".to_string(), "You are the technical lead...".to_string()),
        ("frontend_engineer".to_string(), "Frontend Engineer".to_string(), "You specialize in React...".to_string()),
        ("qa_engineer".to_string(), "QA Engineer".to_string(), "You write comprehensive tests...".to_string()),
    ];
    let team_prompt = "Sequential workflow: Lead designs, Engineer implements, QA validates.";
    let prompt = worker_instructions(1, None, Some(&roles), Some(team_prompt));

    assert!(prompt.contains("## Agent Team"));
    assert!(prompt.contains("Team Objective"));
    assert!(prompt.contains("Sequential workflow"));
    assert!(prompt.contains("Technical Lead"));
    assert!(prompt.contains("Communication Protocol"));
    assert!(prompt.contains("Entropy Reduction"));
    assert!(prompt.contains("Sequential (default)"));
    assert!(prompt.contains("Parallel (broadcast)"));
    assert!(!prompt.contains("Implement with TDD"));
}
```

Update `worker_prompt_without_team_is_solo`:
```rust
let prompt = worker_instructions(1, None, None, None);
```

Update `worker_prompt_with_empty_team_is_solo`:
```rust
let prompt = worker_instructions(1, None, Some(&empty_roles), None);
```

Update `worker_prompt_contains_key_elements`:
```rust
let prompt = worker_instructions(1, Some("/test/worktree"), None, None);
```

**Step 6: Verify**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo build && cargo test`

---

### Task 4: TUI TeamForm — Add team_prompt Field

**Files:**
- Modify: `crates/legion-tui/src/app.rs:402` (team_form_fields type)
- Modify: `crates/legion-tui/src/input.rs:1639-1770` (handle_team_form_keys)
- Modify: `crates/legion-tui/src/ui.rs` (draw_team_form)

**Step 1: Change team_form_fields from [String; 2] to [String; 3]**

In `crates/legion-tui/src/app.rs` line 402:
```rust
pub team_form_fields: [String; 3],    // [name, description, team_prompt]
```

Update `team_form_focus` comment:
```rust
pub team_form_focus: u8,              // 0=name, 1=description, 2=team_prompt, 3=role selection
```

**Step 2: Fix all compilation errors from array size change**

Run `cargo build` to find everywhere that constructs `team_form_fields`. Update all occurrences:

- `App::new()` default: `team_form_fields: [String::new(), String::new(), String::new()],`
- Every place that resets the form fields (search for `team_form_fields` in input.rs):
  - New team: `app.team_form_fields = [String::new(), String::new(), String::new()];`
  - Edit team: `app.team_form_fields = [team.name.clone(), team.description.clone(), team.team_prompt.clone()];`
  - Clone team: same pattern, copy `team_prompt` from source

**Step 3: Update input handler — focus cycling**

In `handle_team_form_keys()`:

The current code has 3 focus zones (0=name, 1=description, 2=roles). We need 4: (0=name, 1=description, 2=team_prompt, 3=roles).

Change the role selection guard from `if app.team_form_focus == 2` to `if app.team_form_focus == 3`:

```rust
fn handle_team_form_keys(app: &mut App, key: KeyEvent) {
    // When in role selection zone (focus == 3), handle navigation and space
    if app.team_form_focus == 3 {
        // ... same Up/Down/Space logic ...
        KeyCode::Tab => {
            app.team_form_focus = 0;
            // ...
        }
        KeyCode::BackTab => {
            app.team_form_focus = 2;  // back to team_prompt
            // ...
        }
        _ => {}
    }
```

Update Tab/BackTab cycling:
```rust
KeyCode::Tab | KeyCode::Down => {
    let next = (app.team_form_focus + 1) % 4;  // 4 zones now
    app.team_form_focus = next;
    if next < 3 {  // text fields 0, 1, 2
        let idx = next as usize;
        app.team_form_cursor = app.team_form_fields[idx].chars().count();
    }
}
KeyCode::BackTab | KeyCode::Up => {
    let next = if app.team_form_focus == 0 { 3 } else { app.team_form_focus - 1 };
    app.team_form_focus = next;
    if next < 3 {
        let idx = next as usize;
        app.team_form_cursor = app.team_form_fields[idx].chars().count();
    }
}
```

Update cursor movement bounds from `< 2` to `< 3`:
```rust
KeyCode::Left => {
    if app.team_form_focus < 3 {
        app.team_form_cursor = app.team_form_cursor.saturating_sub(1);
    }
}
// ... same for Right, Home, End, Backspace, Delete, Char
```

**Step 4: Update save logic**

In the Enter handler, read team_prompt from fields[2]:

```rust
KeyCode::Enter => {
    let name = app.team_form_fields[0].trim().to_string();
    if name.is_empty() { return; }
    let description = app.team_form_fields[1].trim().to_string();
    let team_prompt = app.team_form_fields[2].trim().to_string();
    // ... in team construction, add:
    // team.team_prompt = team_prompt;
```

For editing existing teams:
```rust
team.name = name;
team.description = description;
team.team_prompt = team_prompt;
```

For new teams:
```rust
let team = Team {
    id, name, description, role_ids, is_builtin: false, created_at: now, team_prompt,
};
```

**Step 5: Update draw_team_form in ui.rs**

Add a "Team Prompt:" section between description and role selection. Use the same pattern as existing fields:

- When `focus == 2`: show with cursor (█), editable, word-wrapped
- When `focus != 2`: show as gray text preview (first 3 lines)
- Label: `"Team Prompt:"` in Cyan when focused, White otherwise

The rendering should use `wrap_text()` for multi-line display, similar to how `draw_role_form` handles `prompt_template`.

**Step 6: Verify**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo build && cargo test`

---

### Task 5: Integration Verification

**Step 1: Build and test everything**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo build && cargo test`
Expected: All tests pass.

**Step 2: Verify leader prompt contains dynamic teams**

Add or update a test that verifies `leader_instructions()` with real team data produces the expected table format.

**Step 3: Verify worker prompt contains full framework**

Ensure the `worker_prompt_with_team_roles` test checks for:
- "Team Objective" section
- "Communication Protocol" section
- "Entropy Reduction Rules" section
- "Sequential (default)" and "Parallel (broadcast)" modes
- No TDD workflow when team is active
- TDD workflow when solo

**Step 4: Verify team_prompt round-trips through DB**

Ensure the `team_prompt_persists` test passes — create team with team_prompt, reload, verify it matches.
