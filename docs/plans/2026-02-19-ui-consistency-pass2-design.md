# UI Consistency Pass 2 Design

**Goal:** Unify visual style across all popup screens to match `draw_popup_menu` as the reference standard.

**Approach:** Pure rendering changes in `ui.rs`, no logic or state changes needed.

**Decisions:**
- Selection indicator: `▸` everywhere (replace `>`, `▶`, and mixed usage)
- Footer: Bottom independent area in all list popups (not inline with list items)
- Description format: `Name  -  Description` with 40-char truncation everywhere
- Empty states: All show action hint (e.g., "No teams yet — press [n] to create")
- Border color: Cyan for all popups (replace Green in add_role_to_team)
- set_worker_count: `▸` instead of `▶`

---

## 1. Selection Indicator Unification

**Problem:** Mixed symbols across screens — `▸` (popup_menu, role_list), `>` (manage_teams, team_detail), `▶` (set_worker_count).

**Fix:** Replace all selection indicators with `▸` + consistent spacing `"  "` for unselected items.

**Affected functions:** `draw_manage_teams`, `draw_team_detail`, `draw_set_worker_count`

## 2. Footer Style Unification

**Problem:** Some screens put help hints as the last list item, others use a dedicated bottom area.

**Fix:** All list popups use a dedicated bottom area (like `draw_popup_menu`) with `DarkGray` foreground, separated from list content.

**Affected functions:** `draw_manage_teams`, `draw_team_detail`, `draw_role_list`, `draw_add_role_to_team`

## 3. Description Display Unification

**Problem:** Inconsistent truncation lengths and display format across role/team lists.

**Fix:** All list items use format `"Name  -  Description"` with total line truncated to available width, name capped at 20 chars, description fills remaining space.

**Affected functions:** `draw_manage_teams`, `draw_team_detail`, `draw_role_list`, `draw_add_role_to_team`

## 4. Empty State Messages

**Problem:** Some screens show plain "No items" without action hints.

**Fix:** All empty states show centered message with action hint:
- Teams: `"No teams yet — press [n] to create"`
- Roles: `"No roles yet — press [n] to create"`
- Team roles: `"No roles in team — press [a] to add"`
- Add role: `"(all roles already in team)"`

**Affected functions:** `draw_manage_teams`, `draw_team_detail`, `draw_role_list`

## 5. Border Color Unification

**Problem:** `draw_add_role_to_team` uses Green border, all others use Cyan.

**Fix:** Change to Cyan.

**Affected functions:** `draw_add_role_to_team`

## 6. set_worker_count Indicator

**Problem:** Uses `▶` (filled triangle) instead of `▸` (right-pointing small triangle).

**Fix:** Replace `▶` with `▸`.

**Affected functions:** `draw_set_worker_count`
