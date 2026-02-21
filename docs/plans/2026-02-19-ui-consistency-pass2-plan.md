# UI Consistency Pass 2 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Unify visual style across all popup screens to match `draw_popup_menu`/`draw_main_menu` as the reference standard.

**Architecture:** Pure rendering changes in `ui.rs`. No state or input logic changes needed.

**Tech Stack:** Ratatui (ratatui::layout, ratatui::widgets, ratatui::style)

---

### Task 1: Fix draw_set_worker_count Selection Indicator

**Files:**
- Modify: `crates/legion-tui/src/ui.rs` (line ~2437)

**Step 1:** Find the `▶` character in draw_set_worker_count and replace with `\u{25b8}` (▸).

**Step 2:** Build and verify: `cd /Users/rainlei/holiday/cc_router/legion && cargo build`

---

### Task 2: Fix draw_add_role_to_team Border Color

**Files:**
- Modify: `crates/legion-tui/src/ui.rs` (line ~2956)

**Step 1:** Change `Color::Green` to `Color::Cyan` in the Block border style.

**Step 2:** Build and verify.

---

### Task 3: Unify Footer Layout — draw_manage_teams

**Files:**
- Modify: `crates/legion-tui/src/ui.rs` (draw_manage_teams function, lines ~2464-2535)

**Step 1:** Refactor to use `Layout::default().direction(Direction::Vertical)` with two constraints:
- `Constraint::Min(1)` for the list area
- `Constraint::Length(3)` for the footer area

**Step 2:** Move footer help hints from list items to the dedicated footer chunk. The footer should show key hints like `[n] New  [e] Edit  [d] Delete  [Esc] Back` in `DarkGray` style.

**Step 3:** When `confirm_delete` is active, the footer shows the delete confirmation prompt instead.

**Step 4:** Build and verify.

---

### Task 4: Unify Footer Layout — draw_team_detail

**Files:**
- Modify: `crates/legion-tui/src/ui.rs` (draw_team_detail function, lines ~2537-2629)

**Step 1:** Same layout split as Task 3.

**Step 2:** Move footer hints to dedicated footer chunk: `[a] Add Role  [d] Remove  [Esc] Back`

**Step 3:** Confirm delete prompt goes in footer area.

**Step 4:** Build and verify.

---

### Task 5: Unify Footer Layout — draw_role_list

**Files:**
- Modify: `crates/legion-tui/src/ui.rs` (draw_role_list function, lines ~2752-2826)

**Step 1:** Same layout split.

**Step 2:** Move footer hints to dedicated footer chunk: `[n] New  [e] Edit  [d] Delete  [Esc] Back`

**Step 3:** Confirm delete prompt goes in footer area.

**Step 4:** Build and verify.

---

### Task 6: Unify Footer Layout — draw_add_role_to_team

**Files:**
- Modify: `crates/legion-tui/src/ui.rs` (draw_add_role_to_team function, lines ~2948-3008)

**Step 1:** Same layout split.

**Step 2:** Move footer hints to dedicated footer chunk: `[Space] Toggle  [Enter] Add Selected  [Esc] Cancel`

**Step 3:** Build and verify.

---

### Task 7: Final Build + Test

**Step 1:** `cd /Users/rainlei/holiday/cc_router/legion && cargo build && cargo test`

**Step 2:** Verify no warnings or errors.
