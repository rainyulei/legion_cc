# SDK Kanban + Async Queue + Ralph Loop + Team Mode

## Overview

Workers switch from PTY to SDK execution. Right panel becomes an embedded Task Board (not popup). Tasks use a shared async queue with Ralph Loop iteration and configurable Team modes.

## Architecture

### Task Queue Model

```
Task Queue (N tickets)          Workers (M)
ticket-1  ✓ Done               Worker 1: ticket-4 [▶ iter 2/5]
ticket-2  ✓ Done               Worker 2: ticket-5 [▶ iter 1/5]
ticket-3  ✓ Done               Worker 3: ticket-6 [▶ iter 1/5]
ticket-4  ▶ Working
ticket-5  ▶ Working            Worker 1 completes → picks ticket-7
ticket-6  ▶ Working            Worker 3 completes → picks ticket-8
ticket-7  ○ Queued
...
ticket-N  ○ Queued
```

- Leader submits tickets via MCP tools → orchestrate API
- Workers pick from shared queue (not pre-assigned)
- Worker completes → auto-picks next Queued ticket

### Data Model

```rust
pub struct TaskTicket {
    pub id: usize,
    pub prompt: String,
    pub status: TicketStatus,         // Queued → Working → Done / Error
    pub assigned_worker: Option<u16>,
    pub team_mode: TeamMode,
    pub iteration: u16,               // Current Ralph Loop iteration
    pub max_iterations: u16,          // Default 5
    pub feedback: Option<String>,     // Previous iteration feedback
    pub summary: Option<String>,
    pub elapsed_secs: u64,
}

pub enum TicketStatus { Queued, Working, Done, Error }

pub enum TeamMode {
    TechLeadTeam,    // TL + Engineer + QA (default)
    Solo,            // Single agent, TDD
    Custom(String),  // Custom team description
}
```

### Ralph Loop Execution

```
Worker takes ticket
  │
  Ralph Loop (iteration 1..max_iterations)
  │  Build prompt: iteration 1 = ticket prompt; 2+ = ticket + feedback
  │  Build system prompt: TeamMode → role instructions + TDD + promise requirement
  │  SDK spawn → execute → wait for Result
  │  Check Result text:
  │    Contains <promise>COMPLETE</promise>? → Done ✓
  │    No promise → extract feedback → iteration++ → retry (or Error if max reached)
  └─ Done → Worker idle → picks next ticket
```

### Team System Prompts

**TechLeadTeam**: Tech Lead decomposes → Engineer implements (TDD) → QA validates → TL reviews → promise if pass

**Solo**: Single agent, TDD, outputs promise when done

**Custom**: User-provided team description string

### Layout

```
┌─ Legion v0.1.0 (session-1) [Provider→model] ● W:3 Q:30 ✓12 ▶3 ──┐
│                        │                                           │
│  ┌─ Leader ──────────┐ │ ┌─ Task Board ─────────────────────────┐  │
│  │  Claude Code PTY   │ │ │ ▶ #4  Fix auth       W1 [▶ 2/5]   │  │
│  │  (normal terminal) │ │ │   #5  Validation     W2 [▶ 1/5]   │  │
│  │                    │ │ │   #6  Refactor DB    W3 [▶ 3/5]   │  │
│  │                    │ │ │   ─────────────────                │  │
│  │                    │ │ │   #7  Update docs       ○ Queued   │  │
│  │                    │ │ │   ─────────────────                │  │
│  │                    │ │ │   #1  Setup          ✓ Done 45s    │  │
│  │                    │ │ │   #2  Models         ✓ Done 2m     │  │
│  └────────────────────┘ │ └─────────────────────────────────────┘  │
│  Alt+←→: Focus │ j/k: Select │ Enter: Detail │ Ctrl+P: Menu       │
└────────────────────────────────────────────────────────────────────┘
```

Detail view: Enter on ticket → shows SDK execution log (ANSI formatted via vt100 parser), iteration feedback history, team/elapsed info.

## File Changes

### `legion-core/orchestrate/engine.rs`
- Refactor: shared TaskTicket queue + worker pool
- `submit_ticket(prompt, team_mode)` — Leader adds to queue
- `take_next(worker_id)` — Worker picks next Queued ticket
- `report_iteration(ticket_id, success, feedback)` — SDK result
- `all_tickets()` / `all_workers()` — UI snapshots

### `legion-tui/src/sdk.rs`
- Add promise detection: scan Result text for `<promise>COMPLETE</promise>`
- Extract feedback from non-promise Result text

### `legion-tui/src/app.rs`
- Pane: add `sdk_task`, `sdk_parser`, `sdk_entries` (workers only)
- App: add `right_panel_focused`, ticket list state
- Remove: `show_dashboard`, `capture_pane_screen()`
- New: `start_sdk_task()`, `finish_sdk_task()`, `drain_sdk_entries()`

### `legion-tui/src/lib.rs`
- Remove PTY injection (Ctrl+U + text + Enter)
- Add SDK dispatch: idle worker + queued ticket → spawn SDK
- Add Ralph Loop: SDK done → check promise → retry or complete
- Workers don't spawn PTY in `start_session()`

### `legion-tui/src/ui.rs`
- Right panel: embedded Task Board (list + detail views)
- Header: worker/ticket/progress stats
- Remove: `draw_dashboard_overlay`, popup kanban

### `legion-tui/src/input.rs`
- Remove Ctrl+T
- Right panel focus: j/k navigate tickets, Enter detail, Esc back
- Alt+Left/Right switches Leader↔TaskBoard focus

### `legion-tui/src/claudemd.rs`
- Team mode system prompt templates

### Cleanup
- Delete PTY injection logic (lib.rs)
- Delete `show_dashboard` + Ctrl+T (app/input/ui)
- Delete `capture_pane_screen()` (app)
- Delete popup kanban (ui)
- Delete Worker PTY spawn/resize/write (app/lib)
