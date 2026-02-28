# Agent Teams Integration via tmux Shim

**Date**: 2026-02-28
**Status**: Approved
**Scope**: Plan 1 — Agent Teams Full Coverage

---

## Problem

Legion 需要集成 Claude Code 的原生 Agent Teams 功能，满足三个需求：

1. **在 Leader 中配置和选择 teams/roles**
2. **在 TUI 中为每个 teammate 显示独立 panel 并可交互**
3. **每个 teammate 可代理到不同 API provider/model**

核心挑战：Claude Code Agent Teams 的 tmux 模式会调用 `tmux split-window` 创建真实 tmux pane，但 Legion 使用 ratatui 渲染 TUI，无法直接接收 tmux pane。

---

## Background Research

### Claude Code Agent Teams 的 tmux 交互机制

通过分析 GitHub issues (#23513, #23527, #23615, #24771, #26244) 确认：

- Claude Code **不使用** tmux control mode (`-CC`)
- 仅使用简单 shell 命令：`tmux split-window`、`tmux send-keys`、`tmux list-panes`
- 通过检查 `$TMUX` 环境变量判断是否在 tmux 中
- Agent 间通信通过文件系统（`~/.claude/teams/`、`~/.claude/tasks/`），不经过 tmux
- 每个 teammate 是独立的 `claude` 进程，带 `--agent-name`、`--team-name`、`--parent-session-id` 等参数

### 需要拦截的 tmux 命令

| 命令 | Claude Code 用途 | 频率 |
|---|---|---|
| `split-window [-h\|-v]` | 创建 teammate pane | 每 teammate 一次 |
| `send-keys -t <target> "<cmd>" Enter` | 发送启动命令到 pane | 每 teammate 一次 |
| `list-panes -F '<format>'` | 查询 pane 信息 | 多次 |
| `display-message -p '<format>'` | 查询 session/window 信息 | 偶尔 |
| `capture-pane -p -t <pane>` | 读取 pane 内容 | 偶尔 |
| `-V` | 版本检查 | 启动时一次 |

---

## Solution: Fake tmux Binary (tmux Shim)

### 核心思路

创建一个 Rust 编写的 `tmux` shim 二进制文件，放在 Leader PTY 的 `$PATH` 最前面。当 Claude Code 调用 `tmux` 命令时，shim 拦截并转换为 Legion 的内部操作。

### Architecture

```
┌─────────────────────────────────────────────────┐
│                Legion TUI (ratatui)              │
│                                                  │
│  ┌──────────┐  ┌────────┐ ┌────────┐ ┌────────┐│
│  │  Leader   │  │Teammate│ │Teammate│ │Teammate││
│  │  (PTY)    │  │ 1(PTY) │ │ 2(PTY) │ │ 3(PTY) ││
│  └─────┬─────┘  └───┬────┘ └───┬────┘ └───┬────┘│
│        │             │          │          │     │
│  ┌─────┴─────────────┴──────────┴──────────┴──┐  │
│  │           Shim Controller (in Legion)      │  │
│  │  - Unix socket listener                    │  │
│  │  - Pane registry (id → PTY handle)         │  │
│  │  - Proxy port assignment per teammate      │  │
│  └─────────────────────┬──────────────────────┘  │
│                        │ IPC                     │
│  ┌─────────────────────┴──────────────────────┐  │
│  │         legion-tmux-shim (binary)          │  │
│  │  - Parses tmux CLI args                    │  │
│  │  - Sends JSON request to Unix socket       │  │
│  │  - Returns formatted response to stdout    │  │
│  └────────────────────────────────────────────┘  │
│                                                  │
│  Leader ENV:                                     │
│    TMUX=/tmp/legion-tmux-fake/session,0,0        │
│    PATH=/tmp/legion-shim-bin:$PATH               │
│    CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1        │
└──────────────────────────────────────────────────┘
```

### IPC Protocol

shim 与 Legion 之间通过 Unix domain socket 通信，使用 JSON 协议：

**Request (shim → Legion):**
```json
{
  "command": "split-window",
  "args": ["-h"],
  "session_id": "leader-session"
}
```

**Response (Legion → shim):**
```json
{
  "success": true,
  "pane_id": "%1",
  "pane_index": 1
}
```

**Request: send-keys**
```json
{
  "command": "send-keys",
  "target": "%1",
  "keys": ["claude --agent-name researcher ...", "Enter"]
}
```

**Request: list-panes**
```json
{
  "command": "list-panes",
  "format": "#{pane_index}:#{pane_id}:#{pane_width}:#{pane_height}"
}
```

### Shim Command Handling

```rust
// legion-tmux-shim/src/main.rs (pseudocode)
fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let socket_path = env::var("LEGION_TMUX_SOCKET").unwrap();

    match args.first().map(|s| s.as_str()) {
        Some("split-window") => handle_split_window(&args, &socket_path),
        Some("send-keys")    => handle_send_keys(&args, &socket_path),
        Some("list-panes")   => handle_list_panes(&args, &socket_path),
        Some("display-message") => handle_display_message(&args, &socket_path),
        Some("capture-pane") => handle_capture_pane(&args, &socket_path),
        Some("-V")           => println!("tmux 3.4"),  // fake version
        Some("has-session")  => std::process::exit(0), // always "yes"
        Some("list-sessions") => println!("legion: 1 windows ..."),
        _ => {
            // 未知命令：log warning, 返回成功
            eprintln!("legion-tmux-shim: unhandled command: {:?}", args);
            std::process::exit(0);
        }
    }
}
```

---

## Teammate Pane Lifecycle

### 创建流程

```
1. User enables Agent Teams in Leader conversation
2. Claude Code checks $TMUX → set → chooses tmux mode
3. Claude Code runs: tmux split-window -h
4. Shim intercepts → sends IPC to Legion
5. Legion:
   a. Allocates new proxy port for teammate
   b. Looks up teammate role → resolves provider/model from team config
   c. Creates new PTY pane in TUI
   d. Returns pane_id to shim → shim exits with success
6. Claude Code runs: tmux send-keys -t %1 "claude --agent-name ..." Enter
7. Shim intercepts → sends IPC to Legion
8. Legion:
   a. Parses the claude command to extract --agent-name, --team-name, etc.
   b. Sets pane-specific env vars:
      - ANTHROPIC_BASE_URL=http://127.0.0.1:<teammate-proxy-port>
      - CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1
   c. Spawns the claude command in the teammate PTY
   d. Proxy server on that port routes to the configured provider/model
```

### 销毁流程

```
1. Teammate claude process exits (task complete or error)
2. Legion detects PTY exit
3. Options:
   a. Keep pane visible with final output (default)
   b. Auto-remove pane after delay
   c. User manually closes with keybinding
```

---

## Per-Teammate Proxy Routing

每个 teammate pane 分配独立 proxy port，可路由到不同 provider/model：

```
Leader (PTY)      → proxy:18080 → Anthropic claude-opus-4-6
Teammate 1 (PTY)  → proxy:18081 → OpenRouter claude-sonnet-4-5
Teammate 2 (PTY)  → proxy:18082 → GitHub Copilot gpt-4o
Teammate 3 (PTY)  → proxy:18083 → MiniMax minimax-m2.5
```

### Proxy 配置来源

两种模式：

**A. 跟随 Team 定义（推荐）**

在 TUI Ctrl+P 中配置 team template，每个 role 预设 provider/model：

```json
{
  "team_name": "fullstack",
  "roles": {
    "tech-lead": { "provider": "anthropic", "model": "claude-opus-4-6" },
    "frontend":  { "provider": "openrouter", "model": "claude-sonnet-4-5" },
    "backend":   { "provider": "github_copilot", "model": "claude-sonnet-4-5" },
    "qa":        { "provider": "minimax", "model": "minimax-m2.5" }
  }
}
```

当 shim 拦截到 `send-keys` 中的 `--agent-name` 时，Legion 匹配 role → 分配对应 proxy 配置。

**B. 默认继承 Leader**

如果 team 没有为某 role 配置 provider/model，默认使用 Leader 的配置。

### Role 匹配策略

Claude Code 传递 `--agent-name` 和 `--agent-type`，Legion 用这些做模糊匹配：

```rust
fn resolve_proxy_config(agent_name: &str, agent_type: &str, team_config: &TeamConfig) -> ProxyConfig {
    // 1. 精确匹配 agent_name
    if let Some(cfg) = team_config.roles.get(agent_name) {
        return cfg.proxy.clone();
    }
    // 2. 模糊匹配 agent_type → role
    let role = match agent_type {
        "code-reviewer" | "tech-lead" => "tech-lead",
        "general-purpose" | "engineer" => "engineer",
        "qa" | "tester" => "qa",
        _ => "default",
    };
    team_config.roles.get(role)
        .map(|r| r.proxy.clone())
        .unwrap_or(team_config.default_proxy.clone())
}
```

---

## TUI Layout

### 三栏布局（Agent Teams 活跃时）

```
┌──────────────────┬────────────────┬──────────────┐
│                  │  Teammate 1    │              │
│                  │  [researcher]  │  Task Board  │
│    Leader        │  (interactive) │              │
│    (PTY)         ├────────────────┤  #1 [Done]   │
│                  │  Teammate 2    │  #2 [Working]│
│    You interact  │  [coder]       │  #3 [Queued] │
│    here          │  (interactive) │              │
│                  ├────────────────┤              │
│                  │  Teammate 3    │              │
│                  │  [qa]          │              │
└──────────────────┴────────────────┴──────────────┘
```

- 中间列动态分割，每个 teammate 一个子 pane
- Tab 切换 focus 到不同 teammate pane（可交互输入）
- 每个 teammate pane 顶部显示：agent-name、role、provider/model、状态

### 无 Agent Teams 时（现有布局）

```
┌──────────────────────────┬─────────────────────┐
│                          │   Task Board         │
│    Leader                │                      │
│    (PTY)                 │                      │
├────────┬────────┬────────┤                      │
│Worker 1│Worker 2│Worker 3│                      │
└────────┴────────┴────────┴─────────────────────┘
```

Workers 和 Teammates 共存 — Workers 是 Legion 调度的 SDK 进程，Teammates 是 Claude Code 内部的 Agent Teams 成员。

---

## Team Configuration UX

### Ctrl+P → Teams 菜单

```
┌─────────────────────────────┐
│  Team Configuration         │
├─────────────────────────────┤
│  ▶ Enable Agent Teams  [ON] │
│                             │
│  Active Team: fullstack     │
│                             │
│  Roles:                     │
│    tech-lead  → Opus/Anthropic    │
│    frontend   → Sonnet/OpenRouter │
│    backend    → Sonnet/Copilot    │
│    qa         → M2.5/MiniMax      │
│                             │
│  [Edit Team] [New Team]     │
│  [Delete Team]              │
└─────────────────────────────┘
```

### 配置持久化

- Team 定义存储在 `legion.db`（SQLite）
- 与 session 绑定：每个 session 可以选择不同的 team
- `enable_agent_teams` 标记存储在 session 配置中
- 启用后，下次启动自动恢复

---

## Implementation Components

### 1. `legion-tmux-shim` (新 crate)

独立二进制文件，编译后约 ~1MB：

- `src/main.rs` — 解析 tmux CLI 参数
- `src/ipc.rs` — Unix socket 客户端
- `src/commands/` — 每个 tmux 子命令的处理器

### 2. `legion-core` 变更

- `src/shim/controller.rs` — Unix socket 服务端，处理 shim 请求
- `src/shim/pane_registry.rs` — 管理 teammate pane 状态
- `src/shim/protocol.rs` — IPC JSON 协议定义

### 3. `legion-tui` 变更

- `src/app.rs` — 新增 teammate pane 管理，三栏布局切换
- `src/ui.rs` — 三栏渲染逻辑
- `src/teammate_pty.rs` — teammate PTY 生命周期管理

### 4. `legion-db` 变更

- Team template CRUD
- Session ↔ team binding

---

## Risk & Mitigations

| 风险 | 影响 | 缓解措施 |
|---|---|---|
| Claude Code 更新 tmux 交互方式 | shim 失效 | shim 对未知命令返回成功，保持宽容 |
| `$TMUX` 欺骗导致其他行为变化 | 未知副作用 | 仔细测试 Claude Code 在 fake TMUX 下的所有行为 |
| IPC socket 权限/路径冲突 | 连接失败 | 使用 session-unique socket path |
| teammate 数量不确定 | 端口/pane 耗尽 | 动态分配，设置上限（max 10） |
| Claude Code 的 Agent Teams 仍是 experimental | API 不稳定 | 保持 shim 层薄，快速适配 |

---

## Known Claude Code Agent Teams Bugs (We Fix)

这些是 Claude Code 原生 tmux 模式的已知 bug，fake tmux shim 天然解决：

1. **Race condition** (#23513) — `send-keys` 在 shell 初始化前到达 → shim 直接在 PTY 中执行命令，无 race
2. **pane-base-index** (#23527) — 假设 0-based index → shim 控制 index 映射
3. **Layout destruction** (#23615) — 分裂现有 pane → shim 创建独立 pane，不影响布局
4. **Orphaned panes** (#24771) — pane 断开连接 → Legion 管理完整生命周期

---

## Out of Scope

- 修改 Claude Code 本身的 Agent Teams 行为
- 实现完整的 tmux 协议兼容（只处理 Claude Code 使用的子集）
- Worker（SDK 模式）与 Teammate（Agent Teams）的合并 — 两套共存
- Remote coding（Plan 3）
- Global MCP injection（Plan 2）
