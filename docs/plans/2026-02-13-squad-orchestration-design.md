# Squad Orchestration: Leader-Worker Task Management

## Goal

Enable Leader to plan, dispatch, and verify tasks executed autonomously by Workers. Workers run independently with test criteria, eliminating the need for real-time bidirectional communication.

## Motivation

In squad mode, Leader and Workers currently run as independent Claude Code instances with no coordination. This design adds a lightweight orchestration layer that:

1. Lets Leader break large tasks into tickets and distribute to Workers
2. Workers execute autonomously using TDD verification loops
3. Results flow back via files, Leader verifies at the end
4. Minimizes API calls (no real-time back-and-forth)

## Architecture Overview

```
┌──────────── Legion TUI (Rust) ──────────────────────────────┐
│                                                              │
│  Leader (PTY, interactive)     Workers (PTY, autonomous)     │
│  ┌─────────────────────┐      ┌──────────┐  ┌──────────┐   │
│  │ User interaction     │      │ Worker 1 │  │ Worker 2 │   │
│  │ brainstorm → plan    │      │ ticket A │  │ ticket B │   │
│  │ dispatch tickets     │─────→│ execute  │  │ execute  │   │
│  │ legion-check status  │←─────│ report   │  │ report   │   │
│  │ final verification   │      │ idle     │  │ idle     │   │
│  └─────────────────────┘      └──────────┘  └──────────┘   │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐  │
│  │              Orchestration Engine                       │  │
│  │  • HTTP API (base_port + 2000)                         │  │
│  │  • Task queue per Worker                               │  │
│  │  • Result file management                              │  │
│  │  • PTY injection (one-time per task)                   │  │
│  │  • Worker state tracking                               │  │
│  └────────────────────────────────────────────────────────┘  │
│                                                              │
│  ┌────────────────────────────────────────────────────────┐  │
│  │              TUI Dashboard                              │  │
│  │  • Worker status indicators in pane borders            │  │
│  │  • Completion list overlay (Ctrl+T)                    │  │
│  │  • Progress summary in footer                          │  │
│  └────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────┘
```

## 4-Phase Workflow

### Phase 1: Leader Planning

User gives Leader a high-level task. Leader:

1. **Brainstorm**: Understand requirements, constraints, success criteria
2. **Write plan**: Create implementation plan with clear subtasks
3. **Split tickets**: Break plan into N tickets (N = worker count)
4. **Prepare each ticket** with:
   - Task description (what to implement)
   - Test success criteria (how to verify)
   - Context/background (relevant code, architecture notes)
   - File list (which files to touch)
   - Dependencies (if any, which ticket must complete first)

### Phase 2: Task Distribution

Leader dispatches tickets to Workers:

```
legion-dispatch 1 "ticket content with context and test criteria"
legion-dispatch 2 "ticket content with context and test criteria"
```

Orchestration Engine receives dispatch requests, queues them, and injects ticket text into each Worker's PTY when Worker is idle.

### Phase 3: Worker Autonomous Execution (Parallel)

Each Worker independently:

1. Receives ticket via PTY injection
2. Reads task description and test criteria
3. Implements the code (using Claude Code's normal tools)
4. Runs tests against success criteria
5. If tests fail: iterates (TDD loop) until passing
6. Writes result summary to `/tmp/legion/results/worker-{N}.md`
7. Calls `legion-report done` to signal completion
8. Goes idle

**Workers do NOT communicate with Leader during execution.** They have:
- Clear task description
- Test success criteria for self-verification
- Full autonomy to make implementation decisions

### Phase 4: Monitoring & Verification

TUI displays real-time Worker status:

```
┌─ Squad Dashboard ──────────────────────────────┐
│ Worker 1: ✅ done  "JWT auth"         [3m 12s] │
│ Worker 2: ✅ done  "DB schema"        [2m 45s] │
│ Worker 3: 🔄 working "API endpoints" [4m ...]  │
│                                                  │
│ Completed: 2/3  │  Ctrl+T: toggle dashboard     │
└──────────────────────────────────────────────────┘
```

When all Workers complete (or user requests verification):

1. Leader reads all result files
2. Runs integration tests across all tickets
3. Checks for conflicts between Workers' changes
4. Reports final status to user

## Communication Design

### Principle: One-way, not bidirectional

```
Leader → Worker:  PTY injection (one-time per task)
Worker → Results: File system (/tmp/legion/results/)
Status monitor:   TUI renders + legion-check CLI
```

**No real-time messaging, no notification queues, no blocking calls.**

### CLI Tool Set

| Tool | Direction | Behavior |
|------|-----------|----------|
| `legion-dispatch <id> "ticket"` | Leader → Worker | POST to Orchestrator, injects into Worker PTY |
| `legion-report <status>` | Worker → File | Writes result file, updates Worker status |
| `legion-check` | Leader reads | GET from Orchestrator, prints status board |
| `legion-status` | Any reads | One-line summary of all Workers |
| `legion-stop <id>` | Leader → Worker | Sends interrupt to Worker PTY |
| `legion-stop-all` | Leader → All | Stops all Workers |

These are compiled Rust binaries that communicate with the Orchestration HTTP API.

### Worker Result File Format

Written to `/tmp/legion/results/worker-{N}.md`:

```markdown
---
worker_id: 1
ticket: "Implement JWT authentication middleware"
status: done
duration_seconds: 192
files_modified:
  - src/auth/jwt.rs
  - src/auth/middleware.rs
  - tests/auth_test.rs
test_results: "12/12 passed"
---

## Summary

Implemented JWT authentication middleware with:
- Token generation with 24h expiry
- Middleware for route protection
- Refresh token support

## Test Results

All 12 tests passing:
- test_token_generation
- test_token_validation
- test_expired_token_rejection
...
```

### Orchestration HTTP API

Runs inside the Legion TUI process on `base_port + 2000`:

```
POST /legion/orchestrate/dispatch
  Body: { "worker_id": 1, "ticket": "..." }
  → Queues task, injects into Worker PTY when idle
  → Returns: { "status": "dispatched" }

POST /legion/orchestrate/report
  Body: { "worker_id": 1, "status": "done", "summary": "..." }
  → Updates Worker state, writes result file
  → Returns: { "status": "ok" }

GET /legion/orchestrate/status
  → Returns all Workers' current state
  → { "workers": [{ "id": 1, "status": "working", "ticket": "...", "elapsed": 192 }] }

POST /legion/orchestrate/stop
  Body: { "worker_id": 1 }
  → Sends Ctrl-C to Worker PTY
  → Returns: { "status": "stopped" }

POST /legion/orchestrate/stop-all
  → Stops all Workers
  → Returns: { "status": "all_stopped" }
```

## Worker Configuration

### Worker CLAUDE.md (auto-generated at startup)

```markdown
# Worker {N} - Autonomous Task Executor

You are an autonomous worker. Execute the assigned task using TDD:

1. Read the task description carefully
2. Implement the code
3. Write tests matching the success criteria
4. Run tests until all pass
5. When complete, run: `legion-report done`

Do NOT ask for clarification. Make reasonable decisions based on the task description and codebase context.
```

### Worker Hooks (optional, for status tracking)

```json
{
  "hooks": {
    "Stop": [{
      "hooks": [{
        "type": "command",
        "command": "legion-worker-checkpoint"
      }]
    }]
  }
}
```

`legion-worker-checkpoint` reads the transcript, extracts a brief status, and POSTs to the Orchestration API. This is purely for the TUI dashboard — it does NOT inject tasks or notifications.

## Leader Configuration

### Leader CLAUDE.md (auto-generated at startup)

```markdown
# Squad Leader

You coordinate a team of {N} autonomous Workers.

## Workflow
1. Receive task from user
2. Analyze and create implementation plan
3. Split into tickets (one per worker)
4. Dispatch with: `legion-dispatch <worker_id> "ticket content"`
5. Include in each ticket:
   - Clear task description
   - Test success criteria
   - Relevant file paths and context
6. Monitor with: `legion-check`
7. When all Workers complete, verify integration
8. Report results to user

## Tools
- `legion-dispatch <id> "ticket"` — Send task to Worker
- `legion-check` — View all Workers' status and results
- `legion-status` — Quick one-line status summary
- `legion-stop <id>` / `legion-stop-all` — Emergency stop
```

## PTY Injection

### When to Inject

Orchestrator detects Worker is idle by checking vt100 parser:
- Cursor at prompt position
- Screen shows Claude Code prompt character (❯ or similar)
- No active tool execution

### Injection Sequence

```rust
fn inject_task(pane: &mut Pane, task: &str) {
    pane.pty.write(b"\x15");           // Ctrl-U: clear current line
    pane.pty.write(task.as_bytes());   // Write task text
    pane.pty.write(b"\r");             // Enter: submit
}
```

### First Task vs Subsequent Tasks

- **First task**: Injected after Worker's Claude Code starts and shows prompt
- **Subsequent tasks**: Injected after Worker reports "done" and returns to prompt
- **Emergency stop**: `\x03` (Ctrl-C) sent to Worker PTY

## TUI Dashboard Elements

### Worker Status in Pane Borders

Each Worker pane border shows:
```
┌─ Worker 1 (:18081) [🔄 JWT auth - 3m] ──────────────────┐
```

Status icons:
- 🔄 working (with elapsed time)
- ✅ done
- ❌ error
- ⏸️ idle (waiting for task)

### Completion Overlay (Ctrl+T)

Toggle-able overlay showing all tickets and their status:
```
┌─ Squad Progress ───────────────────────────────────┐
│                                                      │
│  #1 [✅] JWT auth middleware          Worker 1  3m  │
│  #2 [✅] Database schema              Worker 2  2m  │
│  #3 [🔄] API endpoints               Worker 3  4m  │
│  #4 [⏸️] Integration tests           -         -    │
│                                                      │
│  Progress: 2/4 complete                              │
│  Ctrl+T: close                                       │
└──────────────────────────────────────────────────────┘
```

### Footer Status

```
[Provider] Workers: 2/3 done | Ctrl+T: Dashboard | Tab: Focus | Ctrl+Q: Quit
```

## File Structure

```
/tmp/legion/
├── results/
│   ├── worker-1.md      # Worker 1's result
│   ├── worker-2.md      # Worker 2's result
│   └── worker-3.md      # Worker 3's result
├── tickets/
│   ├── worker-1.md      # Worker 1's assigned ticket
│   ├── worker-2.md      # Worker 2's assigned ticket
│   └── worker-3.md      # Worker 3's assigned ticket
└── status.json          # Overall orchestration state
```

## Error Handling

### Worker Fails Task
- Worker encounters unrecoverable error → calls `legion-report error "description"`
- TUI shows ❌ on that Worker's pane
- Leader sees via `legion-check` and decides: reassign, modify ticket, or abort

### Worker Crashes
- PTY process exits unexpectedly
- Orchestrator detects via PTY read error
- TUI shows ❌ with "process exited"
- Leader can restart Worker with new ticket

### Timeout
- Configurable per-ticket timeout (default: 30 minutes)
- If Worker exceeds timeout, Orchestrator sends Ctrl-C
- Worker is marked as timed out
- Leader decides next steps

## Non-Goals (First Version)

- Real-time bidirectional messaging between Leader and Workers
- Worker asking Leader questions (Workers are autonomous)
- Automatic conflict resolution between Workers
- Ticket dependency ordering (all tickets dispatch simultaneously)
- Worker-to-Worker communication
- Automatic retry on failure

## Future Extensions

- **Ticket dependencies**: Orchestrator waits for blocking tickets before dispatching
- **Cross-testing**: Leader dispatches review tasks to other Workers after completion
- **Escalation path**: Worker can call `legion-escalate` for truly blocking issues
- **Shared context**: Workers write to a shared knowledge file that other Workers can read
- **Auto-planning**: Leader automatically plans without user prompting
