# Legion (军团) - 设计文档

## 概述

Legion 是一个 Claude Code 的外壳程序，提供模型切换、Session 管理和小队协作功能。

## 项目信息

| 项 | 值 |
|---|------|
| 名称 | Legion (军团) |
| 命令 | `legion` |
| 技术栈 | Rust + ratatui |
| 位置 | `/Users/rainlei/holiday/cc_router/legion` |
| 关系 | 独立项目，参考 CC Switch 代码逻辑 |

## 核心功能

### 功能 1: 模型切换

- 快捷键 `Ctrl+P` 打开菜单
- 支持 Provider 连接/切换
- 连接后获取 Model List
- 每窗口独立代理端口
- 支持 Anthropic / OpenAI Chat 格式转换

### 功能 2: Session 管理

- 快捷键 `Ctrl+P` → Session 菜单
- 浏览/搜索 Claude Code 会话
- 切换 Session 无需重启
- 支持命名/标记 Session

### 功能 3: 小队模式

- 命令: `legion squad --workers N`
- 1 Leader + N Workers 架构
- tmux 分屏布局
- Unix Socket IPC + SQLite 通信
- 分级策略处理 Worker 问题
  - 低风险: Leader AI 自动决策
  - 高风险: 推送给用户处理

## 架构设计

### 项目结构

```
legion/
├── Cargo.toml              # workspace
├── crates/
│   ├── legion-cli/         # 命令行入口
│   │   └── src/main.rs
│   ├── legion-tui/         # TUI 界面 (ratatui)
│   │   └── src/
│   │       ├── app.rs      # 应用状态
│   │       ├── ui.rs       # 渲染
│   │       ├── input.rs    # 快捷键处理
│   │       ├── popup.rs    # 弹出菜单
│   │       └── pty.rs      # Claude Code 嵌入
│   ├── legion-daemon/      # 后台服务
│   │   └── src/
│   │       ├── server.rs   # Unix Socket Server
│   │       └── router.rs   # 消息路由
│   ├── legion-core/        # 核心逻辑
│   │   └── src/
│   │       ├── proxy/      # HTTP 代理
│   │       ├── session/    # Session 管理
│   │       ├── ipc/        # 进程通信协议
│   │       └── squad/      # 小队模式
│   └── legion-db/          # 数据层
│       └── src/
│           ├── schema.rs   # 表结构
│           └── repo.rs     # CRUD
└── data/
    └── legion.db           # SQLite 数据库
```

### 数据库 Schema

```sql
-- Provider 配置
CREATE TABLE providers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    api_key TEXT,
    api_format TEXT DEFAULT 'anthropic',  -- anthropic / openai_chat
    models TEXT,  -- JSON array (连接后获取)
    is_default INTEGER DEFAULT 0,
    created_at INTEGER
);

-- Session 管理
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    name TEXT,
    project_path TEXT,
    claude_session_file TEXT,  -- 对应 ~/.claude/projects/xxx/*.jsonl
    provider_id TEXT,
    created_at INTEGER,
    last_active_at INTEGER
);

-- 待处理问题队列 (小队模式)
CREATE TABLE pending_questions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    worker_id TEXT NOT NULL,
    risk_level TEXT NOT NULL,  -- low / high
    content TEXT NOT NULL,
    context TEXT,
    status TEXT DEFAULT 'pending',  -- pending / answered / dismissed
    answer TEXT,
    created_at INTEGER,
    answered_at INTEGER
);

-- Worker 状态 (小队模式)
CREATE TABLE workers (
    id TEXT PRIMARY KEY,
    role TEXT NOT NULL,  -- leader / worker
    status TEXT DEFAULT 'idle',  -- idle / busy / waiting / error
    current_task TEXT,
    provider_id TEXT,
    session_id TEXT,
    proxy_port INTEGER,
    pid INTEGER,
    created_at INTEGER,
    updated_at INTEGER
);
```

### 核心流程

```
用户输入: legion start
    ↓
┌─────────────────┐
│ 1. 启动 Daemon  │ ← Unix Socket Server + SQLite
└────────┬────────┘
         ↓
┌─────────────────┐
│ 2. 启动代理     │ ← HTTP Proxy on :18080
└────────┬────────┘
         ↓
┌─────────────────┐
│ 3. 启动 TUI     │ ← ratatui 界面
└────────┬────────┘
         ↓
┌─────────────────┐
│ 4. 内嵌 Claude  │ ← PTY 子进程，ANTHROPIC_BASE_URL=127.0.0.1:18080
└─────────────────┘
```

## UI 设计

### 主界面

```
┌─────────────────────────────────────────────────────────────┐
│ Legion v0.1.0        [GitHub Copilot → claude-opus-4.5] ●  │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  ┌─────────────────────────────────────────────────────────┐│
│  │                                                         ││
│  │  Claude Code 输出区域                                    ││
│  │  (PTY 嵌入)                                             ││
│  │                                                         ││
│  └─────────────────────────────────────────────────────────┘│
│                                                             │
├─────────────────────────────────────────────────────────────┤
│ Ctrl+P: 菜单 │ Ctrl+Q: 退出                                 │
└─────────────────────────────────────────────────────────────┘
```

### Popup 主菜单 (Ctrl+P)

```
┌─────────────────────────────────────────────────────────────┐
│ Legion                                               [ESC] │
├─────────────────────────────────────────────────────────────┤
│  > Provider     [GitHub Copilot ●]                         │
│    Model        [claude-opus-4.5]                          │
│    Session      [legion 开发]                              │
│    ─────────────────────────────────────────────────────── │
│    Settings                                                │
│    Quit                                                    │
└─────────────────────────────────────────────────────────────┘
```

### Provider 子菜单

```
┌─────────────────────────────────────────────────────────────┐
│ Provider                                      [ESC 返回]   │
├─────────────────────────────────────────────────────────────┤
│  > [●] GitHub Copilot                                      │
│    [○] OpenCode Zen Free                                   │
│    [○] OpenAI                                              │
│    ─────────────────────────────────────────────────────── │
│    [+] 添加 Provider                                       │
└─────────────────────────────────────────────────────────────┘
```

### Model 子菜单

```
┌─────────────────────────────────────────────────────────────┐
│ Model [GitHub Copilot]                        [ESC 返回]   │
├─────────────────────────────────────────────────────────────┤
│  > [*] claude-opus-4.5                                     │
│    [ ] claude-sonnet-4.5                                   │
│    [ ] gpt-4o                                              │
│    [ ] o3-mini                                             │
└─────────────────────────────────────────────────────────────┘
```

### Session 子菜单

```
┌─────────────────────────────────────────────────────────────┐
│ Session                                       [ESC 返回]   │
├─────────────────────────────────────────────────────────────┤
│  > [*] legion 开发                                         │
│    [ ] cc-switch 重构 (2h ago)                             │
│    [ ] bug fix #123 (1d ago)                               │
│    ─────────────────────────────────────────────────────── │
│    [+] 新建 Session                                        │
└─────────────────────────────────────────────────────────────┘
```

## 小队模式

### 启动命令

```bash
legion squad --workers 3
```

### tmux 布局

```
┌───────────────────────┬─────────────────────────────────────┐
│                       │           Worker 1                  │
│                       ├─────────────────────────────────────┤
│       Leader          │           Worker 2                  │
│                       ├─────────────────────────────────────┤
│                       │           Worker 3                  │
└───────────────────────┴─────────────────────────────────────┘
```

### Leader 界面

```
┌─────────────────────────────────────────────────────────────┐
│ Legion [Leader]              [GitHub Copilot ●]            │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│  Claude Code ...                                           │
│                                                             │
├─────────────────────────────────────────────────────────────┤
│ [!] 2 个待处理问题                                          │
│ ┌─────────────────────────────────────────────────────────┐│
│ │ [W2] 删除 src/old.rs?              [Y] [N] [V]         ││
│ │ [W3] 覆盖 config.json?             [Y] [N] [V]         ││
│ └─────────────────────────────────────────────────────────┘│
├─────────────────────────────────────────────────────────────┤
│ Ctrl+P: 菜单 │ Tab: 处理问题                                │
└─────────────────────────────────────────────────────────────┘
```

### IPC 消息协议

```rust
enum Message {
    // Worker → Daemon
    WorkerReady { worker_id: String },
    Question { worker_id: String, risk: Risk, content: String },
    StatusUpdate { worker_id: String, status: WorkerStatus },

    // Daemon → Worker
    Answer { question_id: i64, answer: String },
    TaskAssign { task: String },

    // Daemon → Leader
    NewQuestion { question: PendingQuestion },
    WorkerStatusChanged { worker_id: String, status: WorkerStatus },
}

enum Risk {
    Low,   // Leader AI 自动决策
    High,  // 推送给用户
}

enum WorkerStatus {
    Idle,
    Busy,
    Waiting,
    Error,
}
```

## 技术选型

| 组件 | 选择 | 说明 |
|------|------|------|
| TUI 框架 | ratatui | Rust 生态成熟 |
| 数据库 | SQLite | 轻量，单文件 |
| IPC | Unix Socket | 低延迟，可靠 |
| 窗口管理 | tmux | 跨平台，稳定 |
| PTY | portable-pty | 嵌入 Claude Code |
| HTTP 代理 | hyper | 高性能 |

## 开发计划

**Phase 1: 单窗口模式 (MVP)**
1. 项目骨架 + TUI 框架
2. PTY 嵌入 Claude Code
3. HTTP 代理 + Provider 管理
4. Popup 菜单 (Provider/Model/Session)
5. Session 管理

**Phase 2: 小队模式**
1. Daemon + IPC 通信
2. tmux 布局管理
3. 问题队列 + 分级策略
4. Leader 界面增强

## 参考

- CC Switch 代理代码: `cc-switch/src-tauri/src/proxy/`
- OpenCode 交互设计
- ratatui 示例: https://ratatui.rs/examples/
