# Teams/Roles UX Unification Design

**Goal:** Fix all UX inconsistencies and missing features across Teams and Roles CRUD screens.

**Approach:** Incremental fixes on existing code, no architectural refactoring.

**Decisions:**
- Team runs in a single worker (no num_instances)
- TeamForm includes inline role multi-select
- Delete confirmation via inline footer prompt
- Field switching: Tab/Shift+Tab + Up/Down arrows (unified across all forms)
- AddRoleToTeam supports multi-select

---

## 1. TeamForm Cursor + Field Switching

**Problem:** TeamForm has no cursor (append-only), RoleForm has full cursor support. Field switching inconsistent.

**Fix:**
- Add `team_form_cursor: usize` to app state
- Left/Right move cursor, Home/End jump to start/end
- Backspace/Delete operate at cursor position (UTF-8 char boundary safe)
- Tab/Shift+Tab and Up/Down switch fields (both forms)
- On field switch, cursor resets to end of new field

**Files:** `app.rs` (state), `input.rs` (handlers), `ui.rs` (cursor rendering)

## 2. TeamForm Inline Role Multi-Select

**Problem:** Creating a team requires saving first, then navigating to TeamDetail to add roles separately.

**Fix:**
- TeamForm gains a 3rd focus zone (focus=0: name, focus=1: description, focus=2: role list)
- When focus=2: show all available roles with `[x]`/`[ ]` checkboxes
- Space toggles role selection, Up/Down scroll role list
- On save: create team + create all team-role associations
- On edit: show current roles pre-selected, allow toggle

**New state:**
- `team_form_role_selections: Vec<bool>` — parallel to `role_list`
- `team_form_role_scroll: usize` — scroll offset for role list

**Files:** `app.rs`, `input.rs`, `ui.rs`

## 3. Delete Confirmation (Inline Footer)

**Problem:** Delete operations have no confirmation — instant, irreversible.

**Applies to:** Delete Team, Delete Role, Remove Role from Team

**Fix:**
- Add `confirm_delete: Option<(String, DeleteTarget)>` to app state
- `DeleteTarget` enum: `Team(String)`, `Role(String)`, `TeamRole(String, String)`
- Press 'd' → set confirm_delete, footer shows `Delete "NAME"? [y] Yes  [n/Esc] Cancel`
- While confirming: only 'y', 'n', Esc are handled; all other keys ignored
- 'y' → execute delete, clear state; 'n'/Esc → cancel, clear state

**Files:** `app.rs` (state + enum), `input.rs` (handlers), `ui.rs` (footer rendering)

## 4. UI Consistency Fixes

**Truncation:** Unify to 40 chars everywhere (role list, team list, team detail role display)

**Clone indicator:** When editing a cloned builtin, footer shows `(cloning from "ORIGINAL_NAME")`

**Navigation:**
- All forms → Esc → parent list
- All lists → Esc → main menu
- Consistent across RoleList, ManageTeams, TeamDetail, AddRoleToTeam

**Display format:** Unify role/team list item format: `Name  -  Description` with consistent truncation

**Files:** `ui.rs` (rendering), `input.rs` (navigation)

## 5. AddRoleToTeam Multi-Select

**Problem:** Can only add one role at a time.

**Fix:**
- Add `add_role_selections: Vec<bool>` to app state (parallel to filtered role list)
- Space toggles selection, selected shows `[x]`
- Enter adds all selected roles at once
- Footer: `[Space] Toggle  [Enter] Add Selected  [Esc] Cancel`

**Files:** `app.rs`, `input.rs`, `ui.rs`

## 6. Edge Cases

- On field switch: clamp cursor to `min(cursor, new_field.chars().count())`
- Empty lists: show centered "No teams yet — press [n] to create" / "No roles yet — press [n] to create"
- After deleting last item: `selected_index = selected_index.saturating_sub(1)`
- Prevent deleting builtin roles/teams (only custom ones can be deleted; clone first to customize)
