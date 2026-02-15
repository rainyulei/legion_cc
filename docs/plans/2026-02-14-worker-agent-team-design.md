# Worker Agent Team Mode Design

## Goal

每个 Worker 收到任务后，利用 Claude Code 原生 Agent Teams 功能自动创建内部团队（TechLeader / Engineer / QA），在 Ralph Loop 外层保障下迭代完成任务。

## Architecture

```
Legion Leader (用户交互, 正常模式)
  │
  ├─ legion-dispatch → Worker 1 (Ralph Loop + Agent Team)
  │                      │
  │                      └─ Ralph Loop (PROMPT.md 反复喂)
  │                           └─ 每次迭代: Agent Team Lead (delegate mode)
  │                                ├─ TechLeader teammate (拆分 + review)
  │                                ├─ Engineer teammate (实现)
  │                                └─ QA teammate (测试)
  │
  └─ legion-dispatch → Worker 2 (Ralph Loop + Agent Team)
                         └─ (同上)
```

## 双层保障机制

### 外层: Ralph Loop

- 同一个 prompt 反复喂给 Worker
- 每次迭代，Worker 看到之前的文件和 git history
- 提供可靠性保证：即使 Agent Team 会话崩溃或未完成，下一轮自动重试
- TechLeader 确认完成后输出 `<promise>DONE</promise>` 跳出循环

### 内层: Agent Teams

- Worker 本身作为 Agent Team 的 lead，设为 delegate mode（只协调不写代码）
- 固定 3 个 teammates: TechLeader, Engineer, QA
- 使用 in-process 模式（teammates 在同一 PTY 内运行）
- 通过 shared task list 协调工作

## 执行流水线（顺序）

```
1. Worker (team lead) 收到 ticket
2. 创建 Agent Team, spawn 3 teammates
3. TechLeader:
   - 分析 ticket 内容和 success criteria
   - 拆分为最小可测试步骤
   - 创建 shared task list
4. Engineer:
   - 按顺序领取 task
   - 实现代码
5. QA:
   - Engineer 完成一个 task 后立即测试验证
   - 通过 → 标记 task 完成
   - 失败 → message Engineer 返工 → 再测 → 循环
6. 所有 tasks 完成后, TechLeader 最终 review:
   - 对照 success criteria 逐项检查
   - 通过 → 写 summary → Worker lead 输出 <promise>DONE</promise>
   - 不通过 → 创建修复 task → 回到步骤 4
7. Ralph Loop 退出 → legion-report done "summary"
```

## QA 失败处理

QA 发现问题后 message Engineer 返工，Engineer 修复后 QA 再测。在同一个 Agent Team 内循环，不需要退出 Ralph Loop。只有整个 Agent Team 会话意外中断时，Ralph Loop 才会启动下一轮迭代。

## 实现变更

### 1. `pty.rs` — Worker PTY 环境变量

Worker pane 的 PTY 需要新增:
- `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1` — 开启 Agent Teams 功能
- Worker 使用 in-process teammate mode (默认)

仅对 `worker_id.is_some()` 的 pane 设置，Leader 不受影响。

### 2. `claudemd.rs` — 重写 worker_instructions()

从简单的 "TDD 执行" prompt 改为详细的 Agent Team 编排指令:

**Worker lead prompt 需包含:**
- 告知自己是 team lead，使用 delegate mode
- 创建 3 个 teammates 的具体指令和各自 prompt:
  - TechLeader: 分析任务、拆分步骤、最终 review
  - Engineer: 实现代码、响应 QA 反馈
  - QA: 测试验证、反馈问题给 Engineer
- 顺序流水线流程定义
- QA 失败回滚逻辑
- TechLeader 通过后输出 `<promise>DONE</promise>`
- Ralph Loop 退出后调用 `legion-report done "summary"`

### 3. 不需要改动的部分

- **OrchestrateEngine** — Worker 状态追踪不变
- **Legion TUI** — Worker pane 只是内容变丰富
- **legion-tools** — `legion-report` 仍由 Worker lead 调用
- **Ralph Loop 机制** — 已有实现，通过 stop hook 工作

## Teammate 角色定义

### TechLeader

```
你是 TechLeader，负责:
1. 完全理解当前 feature/ticket 的需求和 success criteria
2. 将任务拆分成最小可实现、可测试的步骤
3. 创建 task list 分配给 Engineer 和 QA
4. 所有 task 完成后做最终 review:
   - 逐项检查 success criteria
   - 确认代码质量和测试覆盖
   - 通过则写 summary 通知 lead
   - 不通过则创建修复 task
```

### Engineer

```
你是 Engineer，负责:
1. 按顺序领取 TechLeader 创建的 task
2. 实现代码，遵循 TDD
3. QA 反馈问题时立即修复
4. 每完成一个 task 通知 QA 可以测试
```

### QA

```
你是 QA，负责:
1. Engineer 完成 task 后立即测试验证
2. 运行相关测试用例
3. 验证 edge cases
4. 失败时 message Engineer 说明具体问题
5. 通过时标记 task 完成
```

## 端口和环境

Worker PTY 环境变量:
- `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1` (新增)
- `LEGION_WORKER_ID=N` (已有)
- `LEGION_ORCHESTRATE_PORT=XXXXX` (已有)
- `LEGION_CONTROL_PORT=XXXXX` (已有)
- `ANTHROPIC_BASE_URL=http://127.0.0.1:PORT` (已有, proxy)

## Token 消耗注意

Agent Teams 会显著增加 token 消耗（每个 teammate 独立 context window）。每个 Worker 有 3 个 teammates = 4x token 消耗。对于 2 Worker 的 squad，总共 8 个 Claude Code context windows。适合复杂任务，简单任务建议降 Worker 数。
