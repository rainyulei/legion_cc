# Split-Tickets Skill Design

## Goal

让 Leader Claude 在制定 plan 后，能够系统化地将任务分解为多个 ticket 并批量 dispatch 给 workers，包括正确设置依赖关系（DAG）。

## 机制

采用 **Skill 文件注入** 方案：在 leader worktree 中生成 `.legion/skills/split-tickets.md`，Leader 的 system prompt 中引用该 skill。

## 触发方式

1. **Leader 主动调用**：Leader 识别到需要分解任务时，读取 skill 文件
2. **Plan 后提醒**：Leader 制定 plan 后，system prompt 提示"考虑是否需要用 split-tickets 分解为多个 ticket"

## 文件结构

```
<leader-worktree>/
  .legion/
    skills/
      split-tickets.md    ← 任务分解方法论 + dispatch 模板
```

## 代码变更

### claudemd.rs

1. 新增 `split_tickets_skill() -> String` — 返回 skill 文件内容
2. 新增 `write_leader_skills(leader_worktree: &Path) -> Result<()>` — 写入 `.legion/skills/` 目录
3. 修改 `leader_instructions()` — 在 Workflow 和 Skills 部分加入引用

### app.rs

Squad 启动时调用 `write_leader_skills(&worktree_paths[0])` 生成 skill 文件。

## Skill 文件内容

### Ticket 结构

每个 ticket 包含以下字段：

| 字段 | CLI 标志 | 必填 | 说明 |
|------|----------|------|------|
| worker_id | 位置参数 1 | 是 | 分配给哪个 worker（1-N） |
| title | -t | 是 | 任务目标，描述此 ticket 要完成什么 |
| context | -c | 是 | 技术上下文 + 整体 plan 摘要，让 worker 理解大局和技术栈 |
| criteria | -k | 是 | 可验证的成功标准，每条必须能用命令或检查验证 |
| after | --after | 否 | 依赖的 ticket IDs，逗号分隔 |
| prompt | 最后位置参数 | 是 | 完整任务描述（API 中的 `ticket` 字段，TaskTicket 中的 `prompt` 字段） |

### 字段写作要点

**title (-t)** — 任务目标
- 描述此 ticket 的核心交付物
- 好: "Implement user authentication API"、"Add SQLite persistence layer"
- 差: "Auth" / "Work on stuff"

**context (-c)** — 技术上下文 + plan 概述
- **整体 plan**：当前在做什么、目标是什么、此 ticket 在整体中的位置
- **技术栈**：语言、框架、版本
- **项目结构**：相关目录和文件
- **已有代码**：可复用的接口、类型、函数
- **约束**：性能要求、兼容性、不能用的依赖等
- **注意**：不要包含工作目录路径，Worker 有自己的 worktree

**criteria (-k)** — 成功标准
- 每条标准必须可验证：命令输出、文件存在、API 返回值
- 好: "cargo test passes, POST /login returns 200 with JWT, invalid password returns 401"
- 差: "works correctly" / "good code quality"

**prompt (最后参数)** — 完整实现指导
- Worker 执行任务的全部信息，无需回来问问题
- 具体要创建/修改哪些文件
- API 设计、数据结构、关键逻辑
- 边界情况处理

### 分解流程

1. **分析任务范围** — 理解用户需求，识别功能模块
2. **规划 ticket 划分** — 按文件边界、功能模块拆分，每个 ticket 一个 worker 能独立完成
3. **识别依赖关系** — B 读取 A 创建的文件 → `--after A`；无关则并行
4. **分配 worker** — 用 `legion-status` 查看可用 worker，无依赖的分配给不同 worker
5. **批量 dispatch** — 列出所有命令，逐个执行

### 分解原则

1. **文件边界优先**：不同 ticket 尽量操作不同文件/目录
2. **先基础后上层**：DB schema → API → UI → Tests
3. **最小依赖**：能并行就并行，只在必要时用 `--after`
4. **Context 要充分**：宁多勿少，Worker 看不到 Leader 的上下文
5. **Criteria 要可测**：每条标准必须能用命令验证

## Leader System Prompt 变更

在 Workflow 部分加入：

```
3. Split into tickets — 读取 .legion/skills/split-tickets.md，按照方法论分解任务
```

新增 Skills 部分：

```
## Skills (按需读取)
- `.legion/skills/split-tickets.md` — 任务分解方法论，制定 plan 后读取此文件进行分解和 dispatch
```
