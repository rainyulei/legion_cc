# Squad Board UI Redesign

## Goal

Redesign the right-panel Task Board from a flat todo-list into a proper Kanban Board with structured ticket data, card-style Working items, and a popup detail view showing context, criteria, progress, terminal output, and completion summary.

## Problems with Current Implementation

1. **Not a kanban board** — currently renders as a flat list, looks like a todo list
2. **No meaningful title** — tickets only have `prompt` (raw task description), displayed as `#1 Fix the login...`
3. **Detail requires navigation** — must press Enter to jump into detail view
4. **Missing structured info** — no term context, success criteria, or completion summary displayed

## Design

### Data Structure Changes

`TaskTicket` and `TicketSnapshot` gain three new fields:

| Field | Type | Description |
|-------|------|-------------|
| `title` | `String` | Short task name (required, provided by Leader) |
| `context` | `Option<String>` | Working directory, related files, components |
| `criteria` | `Option<String>` | Success verification conditions |

The existing `prompt` field continues to hold the full task instruction (including team roles, workflow, etc.). The existing `summary` field stores completion details (files changed, commits, approach) after a ticket is Done.

### Submit API Changes

`POST /legion/orchestrate/submit` body:

```json
{
  "title": "Implement OAuth login",
  "ticket": "full prompt with team instructions...",
  "context": "crates/auth/src/",
  "criteria": "OAuth login works, all tests pass, no security holes",
  "team_mode": "tech_lead_team",
  "max_iterations": 5
}
```

Backward compat `/dispatch` endpoint auto-generates title from first 40 chars of ticket.

### Right Panel: Squad Board

```
┌─ Squad Board ──────────────────────┐
│ ▶ WORKING (2)                      │
│ ┌─ #3 Implement auth ──── W1 ────┐ │
│ │ iter 2/5 · 3m12s · TechLead    │ │
│ └────────────────────────────────┘ │
│ ┌─ #5 Fix CSS layout ──── W2 ────┐ │
│ │ iter 1/5 · 0m45s · Solo        │ │
│ └────────────────────────────────┘ │
│                                    │
│ ⏳ QUEUED (3)                      │
│   #6 Add unit tests               │
│   #7 Refactor DB layer            │
│   #8 Update docs                  │
│                                    │
│ ✓ DONE (2)                        │
│   #1 Setup project · 2m30s        │
│   #2 Create models · 4m15s        │
│                                    │
│ ✗ ERROR (0)                       │
└────────────────────────────────────┘
```

- **Working**: bordered card (title + worker + iter/max + elapsed + team_mode)
- **Queued**: compact line (title only)
- **Done**: compact line (title + elapsed)
- **Error**: compact line (title + ERR iter/max)
- Selection cursor (▶) shown when right panel focused, j/k to navigate

### Popup Detail View (Enter on selected card)

```
┌─── #3 Implement auth ─────────────────────────┐
│ Status: Working · W1 · iter 2/5 · 3m12s       │
│ Team: TechLeadTeam                             │
│ Context: crates/auth/src/                      │
│ Criteria: OAuth login works, tests pass        │
│                                                │
│ ─── Progress ──────────────────────────────── │
│ ✅ Write failing test for OAuth                │
│ ✅ Implement OAuth handler                     │
│ ⏳ Running tests...                            │
│                                                │
│ ─── Terminal Output ───────────────────────── │
│ ⏺ Write(src/auth/oauth.rs)                    │
│   pub fn authenticate(token: &str) -> ...      │
│ ⏺ Running: cargo test                         │
│   test auth::test_oauth ... ok                 │
│                                                │
│ ─── Summary (completed tickets) ─────────── │
│ Changed: src/auth/oauth.rs, tests/auth.rs      │
│ Committed: abc1234 "feat: add OAuth login"     │
└─────────────── [Esc: close] ──────────────────┘
```

Three sections:
1. **Header**: status, team, context, criteria
2. **Progress**: structured ProgressEntry items (checkmarks)
3. **Terminal Output**: SDK vt100 parser rendered output
4. **Summary** (Done/Error only): what was done, files changed, commits

### Naming Cleanup

Rename all internal `kanban_*` variables to `board_*`:
- `kanban_selected` → `board_selected`
- `kanban_detail` → `board_detail_open`
- `kanban_detail_scroll` → `board_detail_scroll`
