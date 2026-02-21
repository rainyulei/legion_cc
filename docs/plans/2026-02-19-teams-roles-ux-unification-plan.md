# Teams/Roles UX Unification Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix all UX inconsistencies and missing features across Teams and Roles CRUD screens — cursor support, inline role selection, delete confirmation, multi-select, and UI consistency.

**Architecture:** Incremental fixes to three files (`app.rs`, `input.rs`, `ui.rs`) in `crates/legion-tui/src/`. No new files needed. Each task adds one feature independently.

**Tech Stack:** Rust, ratatui, crossterm, legion-db (SQLite)

---

### Task 1: TeamForm Cursor Support

Add full cursor positioning to TeamForm (matching existing RoleForm cursor behavior).

**Files:**
- Modify: `crates/legion-tui/src/app.rs:394-397` (add state field)
- Modify: `crates/legion-tui/src/input.rs:1572-1638` (handle_team_form_keys)
- Modify: `crates/legion-tui/src/ui.rs:2605-2659` (draw_team_form)

**Step 1: Add `team_form_cursor` to app state**

In `app.rs`, add after line 396 (`team_form_editing`):
```rust
pub team_form_cursor: usize,          // cursor position within current field (char index)
```

In `App::new()`, add after `team_form_editing: None,`:
```rust
team_form_cursor: 0,
```

**Step 2: Initialize cursor in manage_teams_keys**

In `input.rs` `handle_manage_teams_keys`, where `'n'` creates new team (around line 1487), add after `app.team_form_focus = 0;`:
```rust
app.team_form_cursor = 0;
```

Where `'e'` edits team (around line 1499), add after `app.team_form_focus = 0;`:
```rust
app.team_form_cursor = app.team_form_fields[0].chars().count();
```

**Step 3: Rewrite handle_team_form_keys with cursor**

Replace the entire `handle_team_form_keys` function in `input.rs` (lines 1572-1638) with:

```rust
fn handle_team_form_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.mode = AppMode::Popup(PopupMenu::ManageTeams);
        }
        KeyCode::Tab | KeyCode::Down => {
            let next = if app.team_form_focus == 0 { 1 } else { 0 };
            app.team_form_focus = next;
            let idx = app.team_form_focus as usize;
            app.team_form_cursor = app.team_form_fields[idx].chars().count();
        }
        KeyCode::BackTab | KeyCode::Up => {
            let next = if app.team_form_focus == 0 { 1 } else { 0 };
            app.team_form_focus = next;
            let idx = app.team_form_focus as usize;
            app.team_form_cursor = app.team_form_fields[idx].chars().count();
        }
        KeyCode::Left => {
            app.team_form_cursor = app.team_form_cursor.saturating_sub(1);
        }
        KeyCode::Right => {
            let idx = app.team_form_focus as usize;
            let len = app.team_form_fields[idx].chars().count();
            if app.team_form_cursor < len {
                app.team_form_cursor += 1;
            }
        }
        KeyCode::Home => {
            app.team_form_cursor = 0;
        }
        KeyCode::End => {
            let idx = app.team_form_focus as usize;
            app.team_form_cursor = app.team_form_fields[idx].chars().count();
        }
        KeyCode::Enter => {
            let name = app.team_form_fields[0].trim().to_string();
            if name.is_empty() { return; }
            let description = app.team_form_fields[1].trim().to_string();

            if let Ok(repo) = legion_db::open_db() {
                let saved_id;
                if let Some(ref editing_id) = app.team_form_editing {
                    saved_id = editing_id.clone();
                    if let Ok(Some(mut team)) = repo.get_team(editing_id) {
                        team.name = name;
                        team.description = description;
                        let _ = repo.upsert_team(&team);
                    }
                } else {
                    let id = name.to_lowercase().replace(' ', "_");
                    saved_id = id.clone();
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs() as i64;
                    let role_ids = std::mem::take(&mut app.team_form_clone_roles);
                    let team = legion_db::Team {
                        id,
                        name,
                        description,
                        role_ids,
                        is_builtin: false,
                        created_at: now,
                    };
                    let _ = repo.upsert_team(&team);
                }
                app.team_list = repo.list_teams().unwrap_or_default();
                if let Ok(Some(team)) = repo.get_team(&saved_id) {
                    app.team_detail_roles = repo.get_team_roles(&team.id).unwrap_or_default();
                    app.team_detail_team = Some(team);
                    app.team_detail_index = 0;
                    app.mode = AppMode::Popup(PopupMenu::TeamDetail);
                } else {
                    app.mode = AppMode::Popup(PopupMenu::ManageTeams);
                }
            } else {
                app.mode = AppMode::Popup(PopupMenu::ManageTeams);
            }
        }
        KeyCode::Backspace => {
            let idx = app.team_form_focus as usize;
            if app.team_form_cursor > 0 {
                let byte_pos = app.team_form_fields[idx]
                    .char_indices()
                    .nth(app.team_form_cursor - 1)
                    .map(|(i, c)| (i, c.len_utf8()));
                if let Some((start, len)) = byte_pos {
                    app.team_form_fields[idx].replace_range(start..start + len, "");
                    app.team_form_cursor -= 1;
                }
            }
        }
        KeyCode::Delete => {
            let idx = app.team_form_focus as usize;
            let len = app.team_form_fields[idx].chars().count();
            if app.team_form_cursor < len {
                let byte_pos = app.team_form_fields[idx]
                    .char_indices()
                    .nth(app.team_form_cursor)
                    .map(|(i, c)| (i, c.len_utf8()));
                if let Some((start, clen)) = byte_pos {
                    app.team_form_fields[idx].replace_range(start..start + clen, "");
                }
            }
        }
        KeyCode::Char(c) => {
            let idx = app.team_form_focus as usize;
            let byte_pos = app.team_form_fields[idx]
                .char_indices()
                .nth(app.team_form_cursor)
                .map(|(i, _)| i)
                .unwrap_or(app.team_form_fields[idx].len());
            app.team_form_fields[idx].insert(byte_pos, c);
            app.team_form_cursor += 1;
        }
        _ => {}
    }
}
```

**Step 4: Update draw_team_form with cursor rendering**

Replace `draw_team_form` in `ui.rs` (lines 2605-2659) with:

```rust
fn draw_team_form(frame: &mut Frame, app: &App, area: Rect) {
    let title = if app.team_form_editing.is_some() || !app.team_form_clone_roles.is_empty() {
        " Edit Team "
    } else {
        " New Team "
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let field_labels = ["Name", "Description"];
    let mut lines: Vec<Line> = Vec::new();

    for (i, label) in field_labels.iter().enumerate() {
        let is_focused = app.team_form_focus as usize == i;
        let label_style = if is_focused {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        lines.push(Line::from(Span::styled(format!("  {}:", label), label_style)));

        let val = &app.team_form_fields[i];
        if is_focused {
            let cursor = app.team_form_cursor;
            let before: String = val.chars().take(cursor).collect();
            let after: String = val.chars().skip(cursor).collect();
            lines.push(Line::from(Span::styled(
                format!("  {}\u{2588}{}", before, after),
                Style::default().fg(Color::Yellow),
            )));
        } else {
            let max_width = inner.width.saturating_sub(6) as usize;
            let display = truncate_str(val, max_width);
            lines.push(Line::from(Span::styled(
                format!("  {}", display),
                Style::default().fg(Color::Gray),
            )));
        }
        lines.push(Line::from(Span::raw("")));
    }

    lines.push(Line::from(vec![
        Span::styled("  [\u{2190}\u{2192}]", Style::default().fg(Color::Yellow)),
        Span::styled(" Cursor  ", Style::default().fg(Color::Gray)),
        Span::styled("[\u{2191}\u{2193}/Tab]", Style::default().fg(Color::Yellow)),
        Span::styled(" Field  ", Style::default().fg(Color::Gray)),
        Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
        Span::styled(" Save  ", Style::default().fg(Color::Gray)),
        Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
        Span::styled(" Cancel", Style::default().fg(Color::Gray)),
    ]));

    frame.render_widget(Paragraph::new(lines), inner);
}
```

**Step 5: Build and verify**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo build 2>&1 | head -30`
Expected: Compiles successfully

**Step 6: Commit**

```bash
git add crates/legion-tui/src/app.rs crates/legion-tui/src/input.rs crates/legion-tui/src/ui.rs
git commit -m "feat(tui): add cursor support to TeamForm matching RoleForm"
```

---

### Task 2: TeamForm Inline Role Multi-Select

Add a 3rd focus zone to TeamForm showing all roles with checkboxes for inline selection.

**Files:**
- Modify: `crates/legion-tui/src/app.rs` (add state fields)
- Modify: `crates/legion-tui/src/input.rs:handle_team_form_keys` (focus zone 2 handling)
- Modify: `crates/legion-tui/src/ui.rs:draw_team_form` (render role checkboxes)

**Step 1: Add state fields to app.rs**

After `team_form_clone_roles` add:
```rust
pub team_form_role_list: Vec<Role>,       // all available roles for selection
pub team_form_role_selections: Vec<bool>, // parallel to team_form_role_list
pub team_form_role_scroll: usize,         // scroll index in role selection list
```

In `App::new()` add:
```rust
team_form_role_list: Vec::new(),
team_form_role_selections: Vec::new(),
team_form_role_scroll: 0,
```

**Step 2: Initialize role list when entering TeamForm**

In `handle_manage_teams_keys` where `'n'` (new team) is handled, after initializing form fields, add:
```rust
if let Ok(repo) = legion_db::open_db() {
    app.team_form_role_list = repo.list_roles().unwrap_or_default();
    app.team_form_role_selections = vec![false; app.team_form_role_list.len()];
}
app.team_form_role_scroll = 0;
```

Where `'e'` (edit team) is handled, after initializing form fields, add:
```rust
if let Ok(repo) = legion_db::open_db() {
    app.team_form_role_list = repo.list_roles().unwrap_or_default();
    let existing_ids: std::collections::HashSet<&str> = if team.is_builtin {
        app.team_form_clone_roles.iter().map(|s| s.as_str()).collect()
    } else {
        team.role_ids.iter().map(|s| s.as_str()).collect()
    };
    app.team_form_role_selections = app.team_form_role_list.iter()
        .map(|r| existing_ids.contains(r.id.as_str()))
        .collect();
}
app.team_form_role_scroll = 0;
```

**Step 3: Update handle_team_form_keys for 3 focus zones**

Change the Tab/Down handler to cycle through 3 zones (0=name, 1=description, 2=roles):
```rust
KeyCode::Tab | KeyCode::Down => {
    if app.team_form_focus < 2 {
        app.team_form_focus += 1;
    } else {
        app.team_form_focus = 0;
    }
    if app.team_form_focus < 2 {
        let idx = app.team_form_focus as usize;
        app.team_form_cursor = app.team_form_fields[idx].chars().count();
    }
}
KeyCode::BackTab | KeyCode::Up => {
    if app.team_form_focus > 0 {
        app.team_form_focus -= 1;
    } else {
        app.team_form_focus = 2;
    }
    if app.team_form_focus < 2 {
        let idx = app.team_form_focus as usize;
        app.team_form_cursor = app.team_form_fields[idx].chars().count();
    }
}
```

When `focus == 2`, Up/Down navigate role list instead of switching fields, Space toggles:
```rust
KeyCode::Up if app.team_form_focus == 2 => {
    app.team_form_role_scroll = app.team_form_role_scroll.saturating_sub(1);
}
KeyCode::Down if app.team_form_focus == 2 => {
    if app.team_form_role_scroll < app.team_form_role_list.len().saturating_sub(1) {
        app.team_form_role_scroll += 1;
    }
}
KeyCode::Char(' ') if app.team_form_focus == 2 => {
    let idx = app.team_form_role_scroll;
    if idx < app.team_form_role_selections.len() {
        app.team_form_role_selections[idx] = !app.team_form_role_selections[idx];
    }
}
```

**Important:** The `Up`/`Down` guards (`if app.team_form_focus == 2`) must appear BEFORE the generic `Tab`/`Down`/`Up` handlers in the match. Reorder match arms so focus==2 specific arms come first.

On Enter (save), collect selected role_ids:
```rust
// After building name and description, before creating team:
let selected_role_ids: Vec<String> = app.team_form_role_list.iter()
    .zip(app.team_form_role_selections.iter())
    .filter(|(_, &sel)| sel)
    .map(|(r, _)| r.id.clone())
    .collect();
```

For new teams, use `selected_role_ids` as `role_ids`. For editing existing teams, update `team.role_ids = selected_role_ids`.

**Step 4: Update draw_team_form to show role checkboxes**

After drawing the Name and Description fields, when `app.team_form_focus == 2` or always as a section:

```rust
// Role selection section
let roles_label_style = if app.team_form_focus == 2 {
    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
} else {
    Style::default().fg(Color::White)
};
lines.push(Line::from(Span::styled("  Roles:", roles_label_style)));

let visible_roles = 5; // max visible at once
let start = app.team_form_role_scroll.saturating_sub(visible_roles / 2)
    .min(app.team_form_role_list.len().saturating_sub(visible_roles));
let end = (start + visible_roles).min(app.team_form_role_list.len());

for i in start..end {
    let role = &app.team_form_role_list[i];
    let is_selected = app.team_form_role_scroll == i && app.team_form_focus == 2;
    let checked = if app.team_form_role_selections.get(i).copied().unwrap_or(false) {
        "[x]"
    } else {
        "[ ]"
    };
    let prefix = if is_selected { " \u{25b8}" } else { "  " };
    let label = format!("{} {} {}", prefix, checked, role.name);
    let style = if is_selected {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else if app.team_form_role_selections.get(i).copied().unwrap_or(false) {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Gray)
    };
    lines.push(Line::from(Span::styled(label, style)));
}

if app.team_form_role_list.is_empty() {
    lines.push(Line::from(Span::styled(
        "    No roles available",
        Style::default().fg(Color::Gray),
    )));
}
```

Update footer to show Space toggle:
```rust
lines.push(Line::from(vec![
    Span::styled("  [\u{2190}\u{2192}]", Style::default().fg(Color::Yellow)),
    Span::styled(" Cursor  ", Style::default().fg(Color::Gray)),
    Span::styled("[\u{2191}\u{2193}/Tab]", Style::default().fg(Color::Yellow)),
    Span::styled(" Field  ", Style::default().fg(Color::Gray)),
    Span::styled("[Space]", Style::default().fg(Color::Yellow)),
    Span::styled(" Toggle  ", Style::default().fg(Color::Gray)),
    Span::styled("[Enter]", Style::default().fg(Color::Yellow)),
    Span::styled(" Save  ", Style::default().fg(Color::Gray)),
    Span::styled("[Esc]", Style::default().fg(Color::Yellow)),
    Span::styled(" Cancel", Style::default().fg(Color::Gray)),
]));
```

**Step 5: Build and verify**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo build 2>&1 | head -30`

**Step 6: Commit**

```bash
git add crates/legion-tui/src/app.rs crates/legion-tui/src/input.rs crates/legion-tui/src/ui.rs
git commit -m "feat(tui): add inline role multi-select to TeamForm"
```

---

### Task 3: Delete Confirmation (Inline Footer)

Add inline footer confirmation for all delete operations.

**Files:**
- Modify: `crates/legion-tui/src/app.rs` (add DeleteTarget enum + confirm state)
- Modify: `crates/legion-tui/src/input.rs` (wrap delete handlers)
- Modify: `crates/legion-tui/src/ui.rs` (render confirmation footer)

**Step 1: Add DeleteTarget enum and state to app.rs**

After the `MatrixCol` enum, add:
```rust
/// Target for delete confirmation
#[derive(Debug, Clone)]
pub enum DeleteTarget {
    Team(String),          // team id
    Role(String),          // role id
    TeamRole(String, String), // (team_id, role_id)
}
```

Add to App struct after `add_role_index`:
```rust
pub confirm_delete: Option<(String, DeleteTarget)>, // (display_name, target)
```

In `App::new()`:
```rust
confirm_delete: None,
```

**Step 2: Wrap delete in handle_manage_teams_keys**

Replace the `'d'` handler in `handle_manage_teams_keys` (around line 1511):

```rust
KeyCode::Char('d') => {
    if app.confirm_delete.is_some() { return; }
    if app.team_list_index < app.team_list.len() {
        let team = &app.team_list[app.team_list_index];
        app.confirm_delete = Some((
            team.name.clone(),
            crate::app::DeleteTarget::Team(team.id.clone()),
        ));
    }
}
KeyCode::Char('y') => {
    if let Some((_, ref target)) = app.confirm_delete.take() {
        if let crate::app::DeleteTarget::Team(ref id) = target {
            if let Ok(repo) = legion_db::open_db() {
                let _ = repo.delete_team(id);
                app.team_list = repo.list_teams().unwrap_or_default();
            }
            if app.team_list_index >= app.team_list.len() && app.team_list_index > 0 {
                app.team_list_index -= 1;
            }
        }
    }
}
KeyCode::Char('n') if app.confirm_delete.is_some() => {
    app.confirm_delete = None;
}
```

**Step 3: Wrap delete in handle_role_list_keys**

Replace the `'d'` handler in `handle_role_list_keys`:

```rust
KeyCode::Char('d') => {
    if app.confirm_delete.is_some() { return; }
    if app.role_list_index < app.role_list.len() {
        let role = &app.role_list[app.role_list_index];
        if !role.is_builtin {
            app.confirm_delete = Some((
                role.name.clone(),
                crate::app::DeleteTarget::Role(role.id.clone()),
            ));
        }
    }
}
KeyCode::Char('y') => {
    if let Some((_, ref target)) = app.confirm_delete.take() {
        if let crate::app::DeleteTarget::Role(ref id) = target {
            if let Ok(repo) = legion_db::open_db() {
                let _ = repo.delete_role(id);
                app.role_list = repo.list_roles().unwrap_or_default();
            }
            if app.role_list_index >= app.role_list.len() && app.role_list_index > 0 {
                app.role_list_index -= 1;
            }
        }
    }
}
KeyCode::Char('n') if app.confirm_delete.is_some() => {
    app.confirm_delete = None;
}
```

**Step 4: Wrap delete in handle_team_detail_keys**

Replace the `'d'` handler in `handle_team_detail_keys`:

```rust
KeyCode::Char('d') => {
    if app.confirm_delete.is_some() { return; }
    if let Some(ref team) = app.team_detail_team {
        if app.team_detail_index < app.team_detail_roles.len() {
            let role = &app.team_detail_roles[app.team_detail_index];
            app.confirm_delete = Some((
                role.name.clone(),
                crate::app::DeleteTarget::TeamRole(team.id.clone(), role.id.clone()),
            ));
        }
    }
}
KeyCode::Char('y') => {
    if let Some((_, ref target)) = app.confirm_delete.take() {
        if let crate::app::DeleteTarget::TeamRole(ref team_id, ref role_id) = target {
            if let Some(ref mut team) = app.team_detail_team {
                team.role_ids.retain(|id| id != role_id);
                if let Ok(repo) = legion_db::open_db() {
                    let _ = repo.upsert_team(team);
                    app.team_detail_roles = repo.get_team_roles(&team.id).unwrap_or_default();
                }
                if app.team_detail_index >= app.team_detail_roles.len() && app.team_detail_index > 0 {
                    app.team_detail_index -= 1;
                }
            }
        }
    }
}
KeyCode::Char('n') if app.confirm_delete.is_some() => {
    app.confirm_delete = None;
}
```

**Step 5: Clear confirm on Esc in all handlers**

In each handler's `KeyCode::Esc` arm, add `app.confirm_delete = None;` before mode change.

**Step 6: Render confirmation footer in UI**

In each `draw_*` function that has delete, replace the footer section. When `app.confirm_delete.is_some()`, render:

```rust
if let Some((ref name, _)) = app.confirm_delete {
    items.push(ListItem::new(Line::from(vec![
        Span::styled(
            format!("  Delete \"{}\"? ", name),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ),
        Span::styled("[y] ", Style::default().fg(Color::Red)),
        Span::styled("Yes  ", Style::default().fg(Color::Gray)),
        Span::styled("[n/Esc] ", Style::default().fg(Color::Yellow)),
        Span::styled("Cancel", Style::default().fg(Color::Gray)),
    ])));
} else {
    // Normal footer
    // ...existing footer code...
}
```

Apply this pattern to `draw_manage_teams`, `draw_role_list`, and `draw_team_detail`.

**Step 7: Build and verify**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo build 2>&1 | head -30`

**Step 8: Commit**

```bash
git add crates/legion-tui/src/app.rs crates/legion-tui/src/input.rs crates/legion-tui/src/ui.rs
git commit -m "feat(tui): add inline delete confirmation for teams and roles"
```

---

### Task 4: AddRoleToTeam Multi-Select

Change AddRoleToTeam from single-select to multi-select with Space toggle.

**Files:**
- Modify: `crates/legion-tui/src/app.rs` (add selections state)
- Modify: `crates/legion-tui/src/input.rs:handle_add_role_to_team_keys`
- Modify: `crates/legion-tui/src/input.rs:handle_team_detail_keys` (initialize selections)
- Modify: `crates/legion-tui/src/ui.rs:draw_add_role_to_team`

**Step 1: Add state to app.rs**

After `add_role_index`:
```rust
pub add_role_selections: Vec<bool>,  // parallel to add_role_available
```

In `App::new()`:
```rust
add_role_selections: Vec::new(),
```

**Step 2: Initialize selections in handle_team_detail_keys**

Where `'a'` builds add_role_available (around line 1537), after `app.add_role_index = 0;` add:
```rust
app.add_role_selections = vec![false; app.add_role_available.len()];
```

**Step 3: Update handle_add_role_to_team_keys**

Replace the function:

```rust
fn handle_add_role_to_team_keys(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            app.mode = AppMode::Popup(PopupMenu::TeamDetail);
        }
        KeyCode::Up | KeyCode::Char('k') => app.menu_up(),
        KeyCode::Down | KeyCode::Char('j') => app.menu_down(),
        KeyCode::Char(' ') => {
            // Toggle selection
            let idx = app.add_role_index;
            if idx < app.add_role_selections.len() {
                app.add_role_selections[idx] = !app.add_role_selections[idx];
            }
        }
        KeyCode::Enter => {
            // Add all selected roles
            if let Some(ref mut team) = app.team_detail_team {
                let mut added = false;
                for (i, role) in app.add_role_available.iter().enumerate() {
                    if app.add_role_selections.get(i).copied().unwrap_or(false) {
                        team.role_ids.push(role.id.clone());
                        added = true;
                    }
                }
                if added {
                    if let Ok(repo) = legion_db::open_db() {
                        let _ = repo.upsert_team(team);
                        app.team_detail_roles = repo.get_team_roles(&team.id).unwrap_or_default();
                    }
                }
            }
            app.mode = AppMode::Popup(PopupMenu::TeamDetail);
        }
        _ => {}
    }
}
```

**Step 4: Update draw_add_role_to_team with checkboxes**

In the role list rendering, change the label format:

```rust
let checked = if app.add_role_selections.get(i).copied().unwrap_or(false) {
    "[x]"
} else {
    "[ ]"
};
let label = format!("{} {} {}{}{}", prefix, checked, role.name, builtin_tag, desc);
```

Update footer:
```rust
items.push(ListItem::new(Line::from(vec![
    Span::styled("  [Space] ", Style::default().fg(Color::Yellow)),
    Span::styled("Toggle  ", Style::default().fg(Color::Gray)),
    Span::styled("[Enter] ", Style::default().fg(Color::Green)),
    Span::styled("Add Selected  ", Style::default().fg(Color::Gray)),
    Span::styled("[Esc] ", Style::default().fg(Color::Yellow)),
    Span::styled("Back", Style::default().fg(Color::Gray)),
])));
```

**Step 5: Build and verify**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo build 2>&1 | head -30`

**Step 6: Commit**

```bash
git add crates/legion-tui/src/app.rs crates/legion-tui/src/input.rs crates/legion-tui/src/ui.rs
git commit -m "feat(tui): add multi-select to AddRoleToTeam"
```

---

### Task 5: UI Consistency Fixes

Unify truncation, display format, clone indicators, empty list messages, and navigation.

**Files:**
- Modify: `crates/legion-tui/src/ui.rs` (all draw_* functions for teams/roles)
- Modify: `crates/legion-tui/src/input.rs` (navigation consistency)

**Step 1: Unify description truncation to 40 chars**

In `draw_role_list` (line 2680), change `> 30` to `> 40` and `&role.description[..27]` to `&role.description[..37]`:

```rust
let desc_preview = if role.description.len() > 40 {
    format!(" - {}...", &role.description[..37])
} else if !role.description.is_empty() {
    format!(" - {}", role.description)
} else {
    String::new()
};
```

(Note: `draw_add_role_to_team` already uses 40 chars — confirm it matches.)

**Step 2: Add clone indicator to team form footer**

In `draw_team_form`, when `app.team_form_clone_roles` is not empty and `app.team_form_editing` is None, add a line before the footer:

```rust
if app.team_form_editing.is_none() && !app.team_form_clone_roles.is_empty() {
    lines.push(Line::from(Span::styled(
        "  (cloning from builtin team)",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::ITALIC),
    )));
}
```

Similarly in `draw_role_form`, when `app.role_form_clone_source.is_some()`:

```rust
if let Some(ref source) = app.role_form_clone_source {
    lines.push(Line::from(Span::styled(
        format!("  (cloning from \"{}\")", source),
        Style::default().fg(Color::Cyan).add_modifier(Modifier::ITALIC),
    )));
}
```

**Step 3: Improve empty list messages**

In `draw_manage_teams` where empty check is, replace "No teams found" with:
```rust
items.push(ListItem::new(Line::from(Span::styled(
    "  No teams yet \u{2014} press [n] to create",
    Style::default().fg(Color::Gray),
))));
```

In `draw_role_list` where empty check is:
```rust
items.push(ListItem::new(Line::from(Span::styled(
    "  No roles yet \u{2014} press [n] to create",
    Style::default().fg(Color::Gray),
))));
```

In `draw_team_detail` where "No roles assigned":
```rust
items.push(ListItem::new(Line::from(Span::styled(
    "  No roles assigned \u{2014} press [a] to add",
    Style::default().fg(Color::Gray),
))));
```

**Step 4: Build and verify**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo build 2>&1 | head -30`

**Step 5: Commit**

```bash
git add crates/legion-tui/src/ui.rs
git commit -m "fix(tui): unify UI consistency across teams/roles screens"
```

---

### Task 6: Edge Cases

Fix remaining edge cases: cursor clamping, index adjustment after delete, builtin delete prevention.

**Files:**
- Modify: `crates/legion-tui/src/input.rs`

**Step 1: Clamp cursor on field switch**

In both `handle_team_form_keys` and `handle_role_form_keys`, when switching fields, instead of always setting cursor to field end, clamp:

```rust
let idx = app.team_form_focus as usize;
app.team_form_cursor = app.team_form_fields[idx].chars().count();
// This already goes to end, which is the correct UX per design
```

(Already correct — the design says "reset to end of new field".)

**Step 2: Prevent deleting builtin teams**

In `handle_manage_teams_keys` `'d'` handler, add builtin check:

```rust
KeyCode::Char('d') => {
    if app.confirm_delete.is_some() { return; }
    if app.team_list_index < app.team_list.len() {
        let team = &app.team_list[app.team_list_index];
        if !team.is_builtin {
            app.confirm_delete = Some((
                team.name.clone(),
                crate::app::DeleteTarget::Team(team.id.clone()),
            ));
        }
    }
}
```

**Step 3: Add Esc handling for confirm_delete in all screens**

Ensure all `Esc` handlers also clear `confirm_delete`:
```rust
KeyCode::Esc => {
    app.confirm_delete = None;
    // ...existing mode change...
}
```

This applies to: `handle_manage_teams_keys`, `handle_role_list_keys`, `handle_team_detail_keys`.

**Step 4: Build and run tests**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo build && cargo test 2>&1 | tail -20`
Expected: All pass

**Step 5: Commit**

```bash
git add crates/legion-tui/src/input.rs
git commit -m "fix(tui): edge cases — builtin delete prevention, cursor clamping, confirm cleanup"
```

---

### Post-Implementation Verification

After all tasks, manually test:
1. Ctrl+P → main menu → Manage Teams → press 'n' → cursor works in both fields with Left/Right/Home/End
2. Tab into Roles section → Space to select roles → Enter saves team with roles
3. Select team → press 'd' → footer shows confirmation → 'y' deletes, 'n' cancels
4. Manage Roles → select builtin → 'e' → shows clone indicator, edit works
5. Team Detail → 'a' → multi-select with Space → Enter adds all selected
6. Empty team list shows "No teams yet — press [n] to create"
7. Can't delete builtin teams (no confirmation appears)
