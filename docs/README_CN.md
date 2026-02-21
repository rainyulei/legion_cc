# Legion（军团）

**Claude Code 多智能体协同系统** — 让一个 AI 编程助手变成一支协作团队。

> [**English README**](../README.md)

---

## 为什么叫「军团」？

罗马军团是古代世界最高效的作战单位。一个军团长（Legatus）不会自己去挖壕沟 — 他制定战略，把任务拆给百夫长（Centurion），百夫长再分配给士兵并行推进。每个大队有自己的营地（worktree），打完仗统一汇报战果（auto-merge）。

Legion 做的事一模一样，只不过战场换成了你的代码仓库。

## 痛点

你用 Claude Code 写代码，大概率每天都在经历这些：

**上下文压缩吞掉你的细节。** 你花了 20 分钟把需求、架构、边界情况讲得清清楚楚。上下文窗口满了，压缩一启动，一半细节没了。你重新讲，它又忘。来回折腾。

**执行任务把窗口堵死了。** Claude 在那儿写 15 分钟 CRUD，你只能干等着。想聊聊架构？不行，窗口被占了。想审查设计？不行，还在跑。你最贵的 Opus token 和你的注意力，全砸在搬砖上了。

**一讨论细节，设计就丢了。** 本来想好好聊聊架构设计，三条消息之后就在调 import 路径。等你回过神来，设计思路早断了。

**验证全靠你自己盯。** 代码能编译吗？测试过了吗？跟另一个模块能对上吗？没人替你看，每次都得你亲自确认。

**注意力被不停打断。** 文件之间跳来跳去，重读生成的代码，记住哪块做了哪块没做，想清楚下一步干嘛。光是管理 AI 干活的心智负担，就快赶上自己写了。

## Legion 怎么解决

用 Legion，你在 **2-3 轮对话**里把所有任务规划好、发给 worker 并发执行。任务细节不会被压缩丢失，Opus token 留给真正值钱的事 — 设计、架构、拍板。

```
你: "构建用户认证系统"

Leader: 分析 → 拆分为 5 个 ticket → 分发给 3 个 worker

Worker 1: 数据库 schema + 迁移     ──┐
Worker 2: API 端点                  ──┤── 全部并行运行
Worker 3: 密码哈希工具              ──┘
                                      ↓
                自动合并到 leader 分支
                                      ↓
Worker 1: 前端登录表单 (--after 1,2)
Worker 2: 集成测试 (--after 1,2,3)
                                      ↓
                验证检查点: build + test + lint
                                      ↓
Leader: "全部完成。认证系统已集成，测试通过。"
```

串行要 30+ 分钟，并行 10 分钟搞定。

## 核心能力

### 原生 API 代理

内置代理服务器拦截 Claude Code 的 API 流量。你可以接入多个 provider — Anthropic、GitHub Copilot、OpenRouter、MiniMax — 每个 worker 可以单独选择走哪个 provider 和用哪个模型。不需要外部代理，不需要配置文件，装好就能用。

### 多 Worker 并发 + DAG 工作流

多个 worker 同时干活，每个都在自己的 git worktree 里，互相看不到对方的文件。`--after 1,2` 的意思是「等 ticket 1 和 2 做完合并了再启动我」。执行顺序、自动合并、rebase — 引擎全包了。

### Agent Teams — 看得见、管得住

Worker 内部可以组团队：Tech Lead 审方案、Engineer 写代码、QA 跑验证。每个 worker 的终端输出实时可见，你随时能看到谁在干嘛，觉得不对可以随时喊停。

### 多层质量把关

不是写完再补测试，而是从头到尾都有人盯着：

- **QA 角色** — 团队内的质量检查
- **Tech Lead 角色** — 完成前的技术审查
- **Ralph Loop** — 迭代重试，带反馈跑到达标为止
- **检查点 Ticket** — 模块完成后跑 `build + test + lint`，下游任务开始前先确认上游没问题

### 多 Session 管理

干到一半可以存档，下次接着来。不同功能分支之间随意切换。任务看板把所有 worker 的状态摆在面前 — 排队、执行中、完成、出错，加上合并状态、文件 diff、错误详情。

### Worktree 并行隔离

每个 worker 有自己的 git worktree 和分支，写同一个仓库但互不干扰。一个 worker 完成了就立刻合并到 leader 分支，下一个 worker 启动前自动 rebase 拿到最新代码。

## 快速开始

### 安装

```bash
git clone https://github.com/rainyulei/legion_cc.git
cd legion
make build

# 安装到 /usr/local/bin
make install

# 或创建 macOS .pkg 安装包
make pkg
```

### 首次使用

```bash
cd /path/to/your/project    # 必须是 git 仓库

# 初始化（创建 .legion/、CLAUDE.md、.claude/commands/）
legion init

# 启动 TUI（Leader + 2 Workers）
legion
```

Legion 会打开一个分屏终端界面：

```
┌──────────────────────────┬─────────────────────┐
│                          │   任务看板           │
│    Leader                │                      │
│    (Claude Code PTY)     │  #1 Auth API  [完成] │
│                          │  #2 Auth UI [执行中] │
│    在这里正常交互         │  #3 测试    [排队]   │
│                          │     └─ 依赖: 1,2     │
│                          │                      │
├────────┬────────┬────────┤                      │
│Worker 1│Worker 2│Worker 3│                      │
│ (SDK)  │ (SDK)  │ (SDK)  │                      │
└────────┴────────┴────────┴─────────────────────┘
```

### 工作流程

1. **在 Leader pane 和 Claude Code 正常对话**
2. **用 `/split-tickets`** 做任务拆解，规划 DAG 依赖和检查点
3. **Leader 调用 `legion-dispatch` 分发 ticket** — 每个 ticket 带着完整上下文、验收标准、依赖链
4. **Worker 自动并行执行** — worker pane 里能看到实时输出
5. **任务看板追踪全局** — 状态、合并情况、diff、错误信息一目了然
6. **完成的 ticket 自动合并** 到 leader 分支
7. **检查点 ticket 卡住质量门禁** — build、test、lint 全过了才放行下游

### 快捷键

| 按键 | 操作 |
|------|------|
| `Ctrl+P` | 设置菜单（Provider、模型、团队、会话） |
| `Ctrl+Q` | 退出 |
| `Tab` | 在 pane 之间切换焦点 |
| `[` / `]` | 调整 leader/worker 面板分割比例 |
| `j` / `k` | 任务看板上下导航 |
| `Enter` | 查看 ticket 详情 |
| `r` | 重试失败的 ticket |
| `d` | 删除已完成/失败的 ticket |
| `f` | 查看 ticket 的代码 diff |
| `Shift+拖拽` | 复制文本（squad 模式下） |

## 架构

```
┌─────────────────────────────────────────────────────┐
│                    TUI (ratatui)                     │
│  ┌──────────────┐  ┌────────┐ ┌────────┐           │
│  │   Leader      │  │Worker 1│ │Worker 2│  ...      │
│  │  (PTY/Claude) │  │ (SDK)  │ │ (SDK)  │           │
│  └──────┬───────┘  └───┬────┘ └───┬────┘           │
│         │              │          │                  │
│  ┌──────┴──────────────┴──────────┴──────┐          │
│  │         编排引擎 (Orchestration)       │          │
│  │    (Ticket 队列 + DAG 调度器)          │          │
│  └──────────────┬────────────────────────┘          │
│                 │                                    │
│  ┌──────────────┴────────────────────────┐          │
│  │           代理服务器 (Proxy)            │          │
│  │   (Anthropic/OpenAI/Copilot 路由)      │          │
│  └────────────────────────────────────────┘          │
└─────────────────────────────────────────────────────┘
```

### Crates

| Crate | 干嘛的 |
|-------|--------|
| `legion-cli` | CLI 入口，处理命令行参数 |
| `legion-core` | 代理服务器、控制 API、编排引擎 |
| `legion-tui` | TUI 界面（ratatui + tui-term）、PTY 和 SDK 管理 |
| `legion-db` | SQLite 存储（providers、sessions、tickets） |
| `legion-tools` | Leader 用的 MCP 工具集 |

### Git Worktree 隔离

每个 pane 在独立的 worktree 和分支里运行：

```
/my-project/                         ← 主仓库（默认 session 的 Leader）
/my-project-legion/
  session-1/
    leader/                          ← 分支: legion/session-1/leader
    worker-1/                        ← 分支: legion/session-1/worker-1
    worker-2/                        ← 分支: legion/session-1/worker-2
```

## CLI 命令

启动 Legion 用这些：

```bash
# 初始化项目
legion init

# 启动（默认 2 workers，端口 18080）
legion

# 自定义端口
legion --base-port 19080
```

Worker 数量、provider、模型都在 TUI 里通过 `Ctrl+P` 设置，不需要命令行参数。

## Leader pane 内的 MCP 工具

启动后，Leader（就是你正在交互的那个 Claude Code）可以调用这些工具来指挥 worker：

```bash
# 分发任务
legion-dispatch <worker_id> -t "标题" -c "上下文" -k "验收标准" \
  [--after N,M] [--team tech_lead_team] [--plan "..."] "完整描述"

# 查看队列
legion-check

# 状态摘要
legion-status

# 停止任务
legion-stop <ticket_id>
legion-stop all
```

这些不是终端命令，是 Claude Code 在对话中调用的 MCP 工具。你告诉 Leader「把这个任务分给 worker 1」，Leader 就会调用 `legion-dispatch`。

## 最佳实践

### 1. 按文件边界拆分

一个 ticket 操作一组文件，别让两个 ticket 改同一个文件：

```
好: Ticket 1 → src/db/    Ticket 2 → src/api/    Ticket 3 → src/ui/
差: Ticket 1 → src/app.rs  Ticket 2 → src/app.rs  （冲突风险）
```

### 2. 依赖别加多了

- 没有依赖的 ticket → 不加 `--after` → 并行跑
- 「API 要用到 DB 的类型」→ 加 `--after` DB ticket
- 依赖越少，并行度越高

### 3. 模块之间插检查点

每个功能模块做完，加一个检查点 ticket 跑 build + test + lint：

```
T1: 实现数据库 schema
T2: 数据库测试                (--after 1)
T3: 验证数据库模块            (--after 1,2)    ← 检查点
T4: 实现 API 路由             (--after 3)      ← 依赖检查点，不是 T1/T2
```

下游任务依赖检查点而不是直接依赖上游 ticket，这样检查点通不过就不会继续往下跑。

### 4. ticket 里把上下文写全

Worker 看不到你和 Leader 聊了什么，所以 ticket 里要把它需要知道的全写进去：

```bash
legion-dispatch 1 \
  -t "实现用户认证 API" \
  -c "Rust/axum, PostgreSQL via sqlx。User 结构体在 src/db/schema.rs" \
  -k "POST /login 返回 JWT，无效密码返回 401。cargo test 通过。" \
  --plan "文件: src/api/auth.rs (新建), src/api/mod.rs (修改)" \
  "实现登录和注册端点..."
```

### 5. Worker 数量看着来

- 小活（5-8 tickets）→ 2-3 workers
- 中等（10-20 tickets）→ 4-6 workers
- `Ctrl+P → Set Workers` 随时调

## Provider 支持

| 提供商 | 格式 | 认证方式 | 模型 |
|--------|------|----------|------|
| 原生 | `anthropic` | Claude Code 内置 | claude-opus-4-6, claude-sonnet-4-5 |
| GitHub Copilot | `github_copilot` | OAuth 设备授权 | claude-sonnet/opus, gpt-4o, gpt-5.2-codex |
| OpenRouter | `openai_chat` | API Key | OpenRouter 上的任何模型 |
| MiniMax | `openai_chat` | API Key | MiniMax-M2.5, M2.1, M2 |

在 TUI 里 `Ctrl+P → Connect Provider` 添加。每个 worker 可以单独选 provider 和模型 — 通过 `Ctrl+P → Model Matrix` 配置。比如 Leader 用 Opus 做设计，Worker 用 Sonnet 搬砖，省钱。

## 许可证

MIT
