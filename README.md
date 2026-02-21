# Legion

**Multi-agent orchestration for Claude Code** — turn one AI coding assistant into a coordinated squad.

> [**中文文档**](docs/README_CN.md)

---

## Why "Legion"?

Roman legions were the most effective fighting units of the ancient world. A Legatus didn't dig trenches himself — he set strategy, broke tasks down for his Centurions, and they commanded soldiers to push forward in parallel. Each cohort had its own camp (worktree), and after the battle, results were reported back to command (auto-merge).

Legion does the same thing. The battlefield is just your codebase.

## The Pain

You use Claude Code to write code. You probably hit these walls every day:

**Context compression eats your work.** You spend 20 minutes laying out requirements, architecture decisions, edge cases. The context window fills up, compression kicks in, half your carefully crafted details are gone. You explain again. It forgets again. Rinse and repeat.

**Execution hogs your window.** Claude is grinding through CRUD boilerplate for 15 minutes while you sit there watching. Want to discuss architecture? Can't — window's busy. Want to review the design? Nope, still running. Your most expensive resources — Opus tokens and your attention — burned on grunt work.

**Implementation noise drowns out design.** You start a session to think through architecture. Three messages in, you're debugging an import path. By the time you look up, you've lost the design thread entirely.

**Verification is all on you.** Does that code compile? Do the tests pass? Does it integrate with what the other module produced? Nobody's checking but you. Every. Single. Time.

**Your attention gets shredded.** Jumping between files, re-reading generated code, remembering what's done and what isn't, figuring out the next step. The cognitive overhead of managing AI-assisted development is almost as exhausting as doing it yourself.

## How Legion Fixes This

With Legion, you **plan and publish all your tasks in 2-3 context windows**, then workers execute everything concurrently. Your task details never get lost to compression. Your Opus tokens stay on what actually matters — design, architecture, and making decisions.

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

30+ minutes sequentially → under 10 in parallel.

## What's Inside

### Native API Proxy

Built-in proxy server intercepts Claude Code's API traffic. Hook up multiple providers — Anthropic, GitHub Copilot, OpenRouter, MiniMax — and each worker can use a different provider and model. No external proxy, no config files. It just works.

### Concurrent Workers + DAG Workflow

Multiple workers run at the same time, each in its own git worktree, completely isolated. `--after 1,2` means "wait until tickets 1 and 2 are done and merged before starting me." Execution order, auto-merge, rebasing — the engine handles all of it.

### Agent Teams — Visible and Controllable

Workers can form teams internally: Tech Lead reviews the approach, Engineer writes code, QA runs verification. Every worker's terminal output streams in real-time. You see exactly what each one is doing, and you can pull the plug whenever you want.

### Multi-Layer Quality Gates

Quality isn't bolted on at the end. It's wired into every stage:

- **QA roles** run quality checks inside the team workflow
- **Tech Lead roles** do technical review before marking work complete
- **Ralph Loop** retries with feedback until the output meets your standards
- **Checkpoint tickets** run `build + test + lint` at module boundaries — downstream work doesn't start until upstream is verified

### Multi-Session Management

Save your progress, pick it up later. Switch between feature branches. The task board shows every worker's status at a glance — Queued, Working, Done, Error — plus merge state, file diffs, and error details.

### Worktree-Based Parallel Isolation

Each worker gets its own git worktree and branch. They write to the same repo but never touch each other's files. When a worker finishes, its branch merges into the leader branch immediately. The next worker rebases before starting, so it always has the latest code.

## Quick Start

### Install

```bash
git clone https://github.com/rainyulei/legion_cc.git
cd legion
make build

# Install to /usr/local/bin
make install

# Or create macOS .pkg installer
make pkg
```

### First Run

```bash
cd /path/to/your/project    # must be a git repo

# Initialize (creates .legion/, CLAUDE.md, .claude/commands/)
legion init

# Launch TUI (Leader + 2 Workers)
legion
```

Legion opens a split-pane terminal UI:

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

1. **Talk to the Leader pane** — it's a normal Claude Code session
2. **Use `/split-tickets`** to plan task decomposition with DAG dependencies and checkpoints
3. **Leader calls `legion-dispatch`** to send tickets — each one carries full context, acceptance criteria, and dependency chain
4. **Workers execute in parallel** — watch real-time output in worker panes
5. **Task Board tracks everything** — status, merge state, diffs, errors
6. **Completed tickets auto-merge** to the leader branch
7. **Checkpoint tickets gate quality** — build, test, lint must pass before downstream work begins

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

| Crate | What it does |
|-------|-------------|
| `legion-cli` | CLI entry point, parses args |
| `legion-core` | Proxy server, control API, orchestration engine |
| `legion-tui` | Terminal UI (ratatui + tui-term), PTY and SDK management |
| `legion-db` | SQLite storage (providers, sessions, tickets) |
| `legion-tools` | MCP toolset for the Leader |

### Git Worktree Isolation

Each pane runs in its own worktree with a dedicated branch:

```
/my-project/                         ← main repo (Leader in default session)
/my-project-legion/
  session-1/
    leader/                          ← branch: legion/session-1/leader
    worker-1/                        ← branch: legion/session-1/worker-1
    worker-2/                        ← branch: legion/session-1/worker-2
```

## CLI Commands

These are what you type in your terminal to start Legion:

```bash
# Initialize a project
legion init

# Launch (default: 2 workers, port 18080)
legion

# Custom base port
legion --base-port 19080
```

Worker count, providers, and models are all configured inside the TUI via `Ctrl+P`. No command-line flags needed.

## Leader MCP Tools

Once Legion is running, the Leader (the Claude Code session you're chatting with) can call these MCP tools to command workers:

```bash
# Dispatch a ticket
legion-dispatch <worker_id> -t "title" -c "context" -k "criteria" \
  [--after N,M] [--team tech_lead_team] [--plan "..."] "full description"

# Check the queue
legion-check

# Quick status
legion-status

# Stop a ticket or everything
legion-stop <ticket_id>
legion-stop all
```

These aren't terminal commands — they're MCP tools that Claude Code calls during conversation. You tell the Leader "send this task to worker 1" and it calls `legion-dispatch` under the hood.

## Best Practices

### 1. Split by File Boundaries

One ticket, one set of files. Don't have two tickets editing the same file:

```
Good:  Ticket 1 → src/db/    Ticket 2 → src/api/    Ticket 3 → src/ui/
Bad:   Ticket 1 → src/app.rs  Ticket 2 → src/app.rs  (conflict risk)
```

### 2. Keep Dependencies Minimal

- No dependency → no `--after` → runs in parallel
- "API needs DB types" → add `--after` on the DB ticket
- Fewer dependencies = more parallelism

### 3. Put Checkpoints Between Modules

After each functional module, add a checkpoint ticket that runs build + test + lint:

```
T1: Implement DB schema
T2: DB tests                  (--after 1)
T3: Verify DB module          (--after 1,2)    ← checkpoint
T4: Implement API routes      (--after 3)      ← depends on checkpoint, not T1/T2
```

Downstream tickets depend on the checkpoint, not the individual tasks. If the checkpoint fails, nothing moves forward.

### 4. Put Everything in the Ticket

Workers can't see your conversation with the Leader. Write the ticket like you're handing it to a new hire who knows nothing about the project:

```bash
legion-dispatch 1 \
  -t "Implement user auth API" \
  -c "Rust/axum, PostgreSQL via sqlx. User struct in src/db/schema.rs." \
  -k "POST /login returns JWT on valid credentials, 401 on invalid. cargo test passes." \
  --plan "Files: src/api/auth.rs (new), src/api/mod.rs (modify)." \
  "Implement login and register endpoints..."
```

### 5. Scale Workers to Fit

- Small jobs (5-8 tickets) → 2-3 workers
- Medium jobs (10-20 tickets) → 4-6 workers
- `Ctrl+P → Set Workers` to adjust on the fly

## Provider Support

| Provider | Format | Auth | Models |
|----------|--------|------|--------|
| Native | `anthropic` | Claude Code built-in | claude-opus-4-6, claude-sonnet-4-5 |
| GitHub Copilot | `github_copilot` | OAuth device flow | claude-sonnet/opus, gpt-4o, gpt-5.2-codex |
| OpenRouter | `openai_chat` | API key | Any model on OpenRouter |
| MiniMax | `openai_chat` | API key | MiniMax-M2.5, M2.1, M2 |

Add providers via `Ctrl+P → Connect Provider` in the TUI. Each worker can use a different provider and model — set it up in `Ctrl+P → Model Matrix`. For example: Leader on Opus for design work, workers on Sonnet for implementation. Save tokens where it counts.

## License

MIT
