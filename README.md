# Legion

Legion is a TUI-based multi-agent orchestration system for Claude Code. It enables a **Leader + Workers** squad model where a human user collaborates with a Leader agent who delegates tasks to autonomous Worker agents running in parallel.

## Architecture

```
┌─────────────────────────────────────────────────┐
│                    TUI (ratatui)                 │
│  ┌──────────────┐  ┌────────┐ ┌────────┐       │
│  │   Leader      │  │Worker 1│ │Worker 2│  ...  │
│  │  (PTY/Claude) │  │ (SDK)  │ │ (SDK)  │       │
│  └──────┬───────┘  └───┬────┘ └───┬────┘       │
│         │              │          │              │
│  ┌──────┴──────────────┴──────────┴──────┐      │
│  │         Orchestration Engine           │      │
│  │    (Ticket Queue + DAG Scheduler)      │      │
│  └──────────────┬────────────────────────┘      │
│                 │                                │
│  ┌──────────────┴────────────────────────┐      │
│  │           Proxy Server                 │      │
│  │   (Anthropic/OpenAI/Copilot routing)   │      │
│  └────────────────────────────────────────┘      │
└─────────────────────────────────────────────────┘
```

### Crates

| Crate | Purpose |
|-------|---------|
| `legion-cli` | CLI entry point (`legion single`, `legion squad`, `legion switch`) |
| `legion-core` | Proxy server, control API, orchestration engine |
| `legion-tui` | TUI app (ratatui + tui-term), PTY management, SDK integration |
| `legion-db` | SQLite persistence (providers, sessions, tickets, diffs) |
| `legion-tools` | MCP tools for Leader agent (`legion-dispatch`, `legion-check`, `legion-status`, `legion-stop`) |

## Squad Workflow

### Overview

```
User → Leader (Claude Code in PTY)
         │
         ├─ Analyzes user request
         ├─ Creates implementation plan
         ├─ Splits plan into tickets with dependencies
         └─ Dispatches tickets via legion-dispatch
                │
    ┌───────────┼───────────┐
    ▼           ▼           ▼
 Worker 1    Worker 2    Worker 3
 (SDK)       (SDK)       (SDK)
    │           │           │
    ▼           ▼           ▼
 commit      commit      commit
    │           │           │
    └─── auto-merge to leader branch ───┘
```

### Git Worktree Isolation

Each pane runs in an isolated git worktree with its own branch:

```
/my-project/                              ← main repository
/my-project-legion/
  session-1/
    leader/                               ← branch: legion/session-1/leader
    worker-1/                             ← branch: legion/session-1/worker-1
    worker-2/                             ← branch: legion/session-1/worker-2
```

Workers operate in their own directories, so they can read/write files without interfering with each other or the leader.

### Task Lifecycle

```
                   ┌──────────────────────────────────┐
                   │          Ticket Queue             │
                   │                                   │
  legion-dispatch  │  Queued ──(is_ready?)──► Working  │
  ───────────────► │    │                      │       │
                   │    │ blocked_by           │       │
                   │    │ not met              ▼       │
                   │    │              ┌──── Done ◄────┤ promise found
                   │    │              │       │       │
                   │    └──────────────┘   auto-merge  │
                   │                       to leader   │
                   │                                   │
                   │               Error ◄─────────────┤ max retries
                   │            (no merge)             │
                   └──────────────────────────────────┘
```

1. **Queued** — Leader dispatches a ticket. If it has `--after` dependencies, it waits.
2. **Working** — Engine assigns ticket to an idle worker. Worker worktree rebases to leader's latest code first.
3. **Done** — Worker outputs `<promise>DONE</promise>`. Code is auto-merged to leader branch.
4. **Error** — Max retries exceeded. Code stays on worker branch (not merged). Leader decides next steps.

### Task Dependencies (DAG)

Leader can specify dependencies when dispatching:

```bash
# Ticket 1: no dependencies, runs immediately
legion-dispatch 1 -t "Auth API" -c "Rust, axum" -k "endpoints work" "Build auth endpoints"

# Ticket 2: depends on ticket 1
legion-dispatch 2 -t "Auth UI" -c "React" -k "login form works" --after 1 "Build login form"

# Ticket 3: depends on tickets 1 AND 2
legion-dispatch 3 -t "Integration Tests" -c "pytest" -k "all tests pass" --after 1,2 "Write integration tests"
```

The engine only assigns a ticket when ALL its dependencies are Done. This ensures:
- Workers start with the latest code from completed dependencies
- No merge conflicts from overlapping file changes
- Clear execution order for dependent tasks

### Auto-Merge

When a Worker completes a task (Done):

1. **Diff is cached** from the worker's worktree (for display in TUI)
2. **Worker branch is merged** into leader's branch (`git merge --no-ff`)
3. If merge conflicts (rare with proper DAG): merge is aborted, ticket marked as "conflict"

When a Worker starts a new task:

1. **Worker worktree rebases** to leader's latest (`git merge <leader-branch>`)
2. This pulls in all previously merged worker code
3. New `base_commit` is recorded for accurate diff tracking

### Leader Tools

| Command | Purpose |
|---------|---------|
| `legion-dispatch <id> -t "title" -c "ctx" -k "criteria" [--after N,M] "desc"` | Submit a task ticket |
| `legion-check` | View ticket queue with status, dependencies, and merge state |
| `legion-status` | One-line compact summary |
| `legion-stop <id>` / `legion-stop all` | Emergency stop |

### Session Completion

When all tasks are done, the user can "Complete Session" from the TUI menu:
1. All worker branches are merged to the default branch (main/master)
2. Worktrees are cleaned up
3. Session is archived in the database

## Quick Start

```bash
# Build
cargo build --release

# Single agent mode (one Claude Code instance with proxy)
legion single

# Squad mode (Leader + Workers)
legion squad --workers 3

# Switch proxy provider on existing Claude Code session
legion switch
```

## Provider Support

Legion's proxy server routes API requests to multiple providers:

| Provider | Format | Notes |
|----------|--------|-------|
| Anthropic | `anthropic` | Native Claude API |
| OpenAI | `openai_chat` | GPT models via chat completions |
| GitHub Copilot | `github_copilot` | Requires GitHub token |
| OpenRouter | `openai_chat` | Multi-model gateway |

Providers are configured through the TUI settings menu and persisted in SQLite.
