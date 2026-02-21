# Legion

**Multi-agent orchestration for Claude Code** — turn one AI coding assistant into a coordinated squad.

Legion wraps Claude Code in a terminal UI that lets a **Leader agent** delegate tasks to multiple **Worker agents** running in parallel, each in its own git worktree. Workers execute autonomously, auto-merge results back, and the Leader coordinates the whole effort — while you stay in the loop.

> [**中文文档**](docs/README_CN.md)

---

## Why Legion?

Claude Code is powerful, but it works one task at a time. When you have a complex feature — database layer, API routes, frontend, tests — you wait for each piece sequentially.

Legion changes this:

```
You: "Build a user auth system"

Leader: Analyzes → Splits into 5 tickets → Dispatches to 3 workers

Worker 1: DB schema + migrations     ──┐
Worker 2: API endpoints               ──┤── all running in parallel
Worker 3: Password hashing utilities  ──┘
                                        ↓
              Auto-merge to leader branch
                                        ↓
Worker 1: Frontend login form (--after 1,2)
Worker 2: Integration tests (--after 1,2,3)
                                        ↓
              Checkpoint: build + test + lint
                                        ↓
Leader: "All done. Auth system is integrated and tests pass."
```

What would take 30+ minutes sequentially finishes in under 10.

## What Makes Legion Different

- **Parallel execution with isolation** — each worker gets its own git worktree and branch. No file conflicts, no stepping on each other's work.
- **DAG-based task scheduling** — `--after 1,2` means "don't start until tickets 1 and 2 are done and merged." The engine enforces execution order automatically.
- **Auto-merge pipeline** — when a worker finishes, code merges to the leader branch immediately. The next worker rebases before starting, so it always has the latest code.
- **Retry with feedback** — if a worker fails, it retries automatically (up to N times). You can also manually retry with additional feedback.
- **Team roles** — workers can internally delegate to specialized roles (Tech Lead → Engineer → QA) for structured workflows.
- **Multi-provider proxy** — route different panes through different API providers (Anthropic, GitHub Copilot, OpenRouter, MiniMax) with per-pane model selection.
- **Session management** — save and resume work across sessions, switch between feature branches, complete and merge when done.

## Quick Start

### Install

```bash
# Clone and build
git clone https://github.com/anthropics/legion.git
cd legion
make build

# Install binaries to /usr/local/bin
make install

# Or create macOS .pkg installer
make pkg
```

### First Run

```bash
cd /path/to/your/project    # must be a git repo

# Initialize Legion in your project
legion init

# Launch squad mode (default: 2 workers)
legion
```

Legion opens a split-pane TUI:

```
┌──────────────────────────┬─────────────────────┐
│                          │   Task Board         │
│    Leader                │                      │
│    (Claude Code PTY)     │  #1 Auth API  [Done] │
│                          │  #2 Auth UI [Working]│
│    You interact here     │  #3 Tests   [Queued] │
│    as usual              │     └─ after: 1,2    │
│                          │                      │
├────────┬────────┬────────┤                      │
│Worker 1│Worker 2│Worker 3│                      │
│ (SDK)  │ (SDK)  │ (SDK)  │                      │
└────────┴────────┴────────┴─────────────────────┘
```

### Workflow

1. **Talk to the Leader** in the left pane — it's a normal Claude Code session
2. **Use `/split-tickets`** to plan task decomposition
3. **Leader dispatches tickets** via `legion-dispatch` with titles, context, criteria, and dependencies
4. **Workers execute in parallel** — you can watch their progress in the worker panes
5. **Task Board shows status** — Queued → Working → Done/Error with merge state
6. **Results auto-merge** to the leader branch as workers complete

### Key Bindings

| Key | Action |
|-----|--------|
| `Ctrl+P` | Settings menu (providers, models, teams, sessions) |
| `Ctrl+Q` | Quit |
| `Tab` | Cycle focus between panes |
| `[` / `]` | Resize leader/worker panel split |
| `j` / `k` | Navigate task board |
| `Enter` | View ticket details |
| `r` | Retry failed ticket |
| `d` | Delete completed/failed ticket |
| `f` | View file diff for ticket |
| `Shift+Drag` | Copy text (in squad mode) |

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                    TUI (ratatui)                     │
│  ┌──────────────┐  ┌────────┐ ┌────────┐           │
│  │   Leader      │  │Worker 1│ │Worker 2│  ...      │
│  │  (PTY/Claude) │  │ (SDK)  │ │ (SDK)  │           │
│  └──────┬───────┘  └───┬────┘ └───┬────┘           │
│         │              │          │                  │
│  ┌──────┴──────────────┴──────────┴──────┐          │
│  │         Orchestration Engine           │          │
│  │    (Ticket Queue + DAG Scheduler)      │          │
│  └──────────────┬────────────────────────┘          │
│                 │                                    │
│  ┌──────────────┴────────────────────────┐          │
│  │           Proxy Server                 │          │
│  │   (Anthropic/OpenAI/Copilot routing)   │          │
│  └────────────────────────────────────────┘          │
└─────────────────────────────────────────────────────┘
```

### Crates

| Crate | Purpose |
|-------|---------|
| `legion-cli` | CLI entry point and commands |
| `legion-core` | Proxy server, control API, orchestration engine |
| `legion-tui` | TUI app (ratatui + tui-term), PTY/SDK management |
| `legion-db` | SQLite persistence (providers, sessions, tickets) |
| `legion-tools` | MCP tools for Leader (`legion-dispatch`, `legion-check`, `legion-status`, `legion-stop`) |

### Git Worktree Isolation

Each pane runs in its own worktree with a dedicated branch:

```
/my-project/                         ← main repository (Leader in default session)
/my-project-legion/
  session-1/
    leader/                          ← branch: legion/session-1/leader
    worker-1/                        ← branch: legion/session-1/worker-1
    worker-2/                        ← branch: legion/session-1/worker-2
```

Workers never touch each other's files. When a worker completes, its branch merges into the leader branch. The next worker rebases before starting, pulling in all prior work.

## Best Practices

### 1. Split by File Boundaries

Each ticket should operate on different files/directories. This minimizes merge conflicts:

```
Good:  Ticket 1 → src/db/    Ticket 2 → src/api/    Ticket 3 → src/ui/
Bad:   Ticket 1 → src/app.rs  Ticket 2 → src/app.rs  (conflict risk)
```

### 2. Use DAG Dependencies Wisely

- Independent tickets → no `--after` → run in parallel
- "API needs DB types" → `--after` the DB ticket
- Don't over-constrain — minimize dependencies to maximize parallelism

### 3. Insert Verification Checkpoints

After each functional module, add a checkpoint ticket:

```
T1: Implement DB schema
T2: Add DB tests           (--after 1)
T3: Verify DB integration  (--after 1,2)    ← checkpoint: build + test + lint
T4: Implement API routes   (--after 3)      ← depends on checkpoint, not T1/T2
```

Checkpoint tickets run build/test/lint and fix any integration issues before subsequent modules begin.

### 4. Provide Rich Context

Workers can't see the Leader's conversation. Include everything they need:

```bash
legion-dispatch 1 \
  -t "Implement user auth API" \
  -c "Rust/axum, PostgreSQL via sqlx. See src/db/schema.rs for User struct." \
  -k "POST /login returns JWT on valid credentials, 401 on invalid. cargo test passes." \
  --plan "Files: src/api/auth.rs (new), src/api/mod.rs (modify). Use existing DB pool from src/db/pool.rs." \
  "Implement login and register endpoints..."
```

### 5. Scale Workers to Task Count

- 2-3 workers for small features (5-8 tickets)
- 4-6 workers for medium features (10-20 tickets)
- Use `Ctrl+P → Set Workers` to scale dynamically

## Provider Support

| Provider | Format | Auth | Models |
|----------|--------|------|--------|
| Native | `anthropic` | Claude Code built-in | claude-opus-4-6, claude-sonnet-4-5 |
| GitHub Copilot | `github_copilot` | OAuth device flow | claude-sonnet/opus, gpt-4o, gpt-5.2-codex |
| OpenRouter | `openai_chat` | API key | Any model on OpenRouter |
| MiniMax | `openai_chat` | API key | MiniMax-M2.5, M2.1, M2 |

Configure providers through `Ctrl+P → Connect Provider` in the TUI. Each pane can use a different provider/model — configure via `Ctrl+P → Model Matrix`.

## CLI Reference

```bash
# Initialize Legion in a project
legion init

# Launch (defaults to squad mode with 2 workers)
legion

# Squad mode with custom worker count and port
legion squad --workers 4 --base-port 18080

# Single agent mode (one Claude Code + proxy, no workers)
legion single

# Switch provider on existing Claude Code session
legion switch
```

### Leader Tools (available inside Leader pane)

```bash
# Dispatch a task ticket
legion-dispatch <worker_id> -t "title" -c "context" -k "criteria" \
  [--after N,M] [--team tech_lead_team] [--plan "..."] "full description"

# Check ticket queue
legion-check

# Quick status summary
legion-status

# Stop a ticket or all tickets
legion-stop <ticket_id>
legion-stop all
```

## License

MIT
