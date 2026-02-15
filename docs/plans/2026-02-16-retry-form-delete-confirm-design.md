# Retry Form & Delete Confirm Design

## Retry Form

Press `r` on an Error ticket opens a single-page popup form with 4 editable fields:
- **Prompt** — pre-filled with original prompt, editable
- **Context** — pre-filled with original context, editable
- **Criteria** — pre-filled with original criteria, editable
- **Feedback** — new field, initially empty, injected into worker prompt

Navigation: Tab/Shift+Tab to switch fields, Enter to confirm retry, Esc to cancel.

On retry: preserve old ticket logs, append new logs with `--- Retry #N ---` separator.

Engine `retry_ticket` updated to accept optional new prompt/context/criteria/feedback values.

## Delete Confirm

Press `d` shows confirmation popup with ticket title. Enter confirms, Esc cancels.
Press `D` shows confirmation popup with count of completed/failed tickets. Enter confirms, Esc cancels.

## New PopupMenu Variants

- `RetryForm` — 4-field edit form
- `DeleteConfirm` — single ticket delete confirmation
- `ClearConfirm` — batch clear confirmation

## New App State

- `retry_form_fields: [String; 4]` (prompt, context, criteria, feedback)
- `retry_form_focus: u8` (0-3)
- `retry_target_id: usize`
- `delete_confirm_id: usize`
