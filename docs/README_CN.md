# Legion

**Claude Code 多智能体协同系统** — 让一个 AI 编程助手变成一支协作团队。

Legion 将 Claude Code 封装在终端界面中，让 **Leader 智能体** 将任务分发给多个并行运行的 **Worker 智能体**，每个 Worker 都在独立的 git worktree 中工作。Worker 自主执行任务、自动合并代码，Leader 统筹全局 — 而你始终掌控大局。

> [**English README**](../README.md)

---

## 为什么做 Legion？

Claude Code 很强大，但它一次只能做一件事。当你有一个复杂功能 — 数据库层、API 路由、前端、测试 — 你只能等它一个一个做完。

Legion 改变了这一点：

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

原本需要 30+ 分钟的串行任务，10 分钟内完成。

## 特色功能

- **并行执行 + 隔离** — 每个 worker 有独立的 git worktree 和分支，互不干扰
- **DAG 任务调度** — `--after 1,2` 表示"等 ticket 1 和 2 完成并合并后再开始"，引擎自动控制执行顺序
- **自动合并流水线** — worker 完成后代码立即合并到 leader 分支，下一个 worker 启动前自动 rebase 获取最新代码
- **失败重试** — worker 失败后自动重试（最多 N 次），也可以手动添加反馈后重试
- **团队角色** — worker 内部可以委派给专业角色（Tech Lead → Engineer → QA），实现结构化工作流
- **多 Provider 代理** — 不同 pane 可以路由到不同的 API 提供商（Anthropic、GitHub Copilot、OpenRouter、MiniMax），支持逐 pane 选择模型
- **会话管理** — 保存和恢复工作进度，切换功能分支，完成后合并

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

# 初始化 Legion（创建 .legion/、CLAUDE.md、.claude/commands/）
legion init

# 启动（打开 TUI，Leader + 2 Workers）
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

1. **在左侧 pane 与 Leader 交谈** — 和正常使用 Claude Code 一样
2. **使用 `/split-tickets`** 进行任务拆解规划
3. **Leader 通过 `legion-dispatch` 分发 ticket** — 包含标题、上下文、验收标准和依赖关系
4. **Worker 并行执行** — 在 worker pane 中查看实时进度
5. **任务看板显示状态** — 排队 → 执行中 → 完成/错误，以及合并状态
6. **结果自动合并** 到 leader 分支

### 快捷键

| 按键 | 操作 |
|------|------|
| `Ctrl+P` | 设置菜单（Provider、模型、团队、会话） |
| `Ctrl+Q` | 退出 |
| `Tab` | 在 pane 之间切换焦点 |
| `[` / `]` | 调整 leader/worker 面板分割比例 |
| `j` / `k` | 在任务看板中上下导航 |
| `Enter` | 查看 ticket 详情 |
| `r` | 重试失败的 ticket |
| `d` | 删除已完成/失败的 ticket |
| `f` | 查看 ticket 的代码 diff |
| `Shift+拖拽` | 复制文本（squad 模式下） |

## 最佳实践

### 1. 按文件边界拆分

每个 ticket 应操作不同的文件/目录，减少合并冲突：

```
好: Ticket 1 → src/db/    Ticket 2 → src/api/    Ticket 3 → src/ui/
差: Ticket 1 → src/app.rs  Ticket 2 → src/app.rs  （冲突风险）
```

### 2. 合理使用 DAG 依赖

- 无依赖的 ticket → 不加 `--after` → 并行执行
- "API 需要 DB 的类型" → `--after` DB ticket
- 不要过度约束 — 最小化依赖以最大化并行度

### 3. 插入验证检查点

每个功能模块完成后，添加一个检查点 ticket：

```
T1: 实现数据库 schema
T2: 添加数据库测试           (--after 1)
T3: 验证数据库模块集成       (--after 1,2)    ← 检查点: build + test + lint
T4: 实现 API 路由            (--after 3)      ← 依赖检查点，而非 T1/T2
```

### 4. 提供充分的上下文

Worker 看不到 Leader 的对话内容，需要在 ticket 中包含所有必要信息：

```bash
legion-dispatch 1 \
  -t "实现用户认证 API" \
  -c "Rust/axum, PostgreSQL via sqlx。User 结构体在 src/db/schema.rs" \
  -k "POST /login 返回 JWT，无效密码返回 401。cargo test 通过。" \
  --plan "文件: src/api/auth.rs (新建), src/api/mod.rs (修改)" \
  "实现登录和注册端点..."
```

### 5. 根据任务量调整 Worker 数

- 小功能（5-8 tickets）→ 2-3 workers
- 中等功能（10-20 tickets）→ 4-6 workers
- 使用 `Ctrl+P → Set Workers` 动态调整

## Provider 支持

| 提供商 | 格式 | 认证方式 | 模型 |
|--------|------|----------|------|
| 原生 | `anthropic` | Claude Code 内置 | claude-opus-4-6, claude-sonnet-4-5 |
| GitHub Copilot | `github_copilot` | OAuth 设备授权 | claude-sonnet/opus, gpt-4o, gpt-5.2-codex |
| OpenRouter | `openai_chat` | API Key | OpenRouter 上的任何模型 |
| MiniMax | `openai_chat` | API Key | MiniMax-M2.5, M2.1, M2 |

通过 TUI 中的 `Ctrl+P → Connect Provider` 配置。每个 pane 可以使用不同的 provider/模型 — 通过 `Ctrl+P → Model Matrix` 设置。

## 许可证

MIT
