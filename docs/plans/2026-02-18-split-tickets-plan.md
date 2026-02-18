# Split-Tickets Skill Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 让 Leader Claude 能通过 `/split-tickets` 命令系统化地分解任务并批量 dispatch tickets

**Architecture:** 两部分：(1) `.claude/commands/split-tickets.md` command 文件提供方法论 (2) leader system prompt 提供触发时机提示

**Tech Stack:** Rust (claudemd.rs, app.rs), Markdown

---

### Task 1: 创建 split-tickets command 内容函数

**Files:**
- Modify: `crates/legion-tui/src/claudemd.rs`

**Step 1: 在 `worker_instructions()` 之后、`write_squad_claude_md()` 之前，添加函数**

```rust
/// Generate the split-tickets command content for .claude/commands/
pub fn split_tickets_command(worker_count: u16) -> String {
    format!(
        r#"# Split Tickets — 任务分解与批量 Dispatch

## 当前团队

你有 {worker_count} 个 Worker 可用。用 `legion-status` 查看实时状态。

## Ticket 结构

每个 ticket 通过 `legion-dispatch` 提交：

```
legion-dispatch <worker_id> -t "<title>" -c "<context>" -k "<criteria>" [--after N,M] "<prompt>"
```

| 字段 | CLI 标志 | 必填 | 说明 |
|------|----------|------|------|
| worker_id | 位置参数 1 | 是 | 分配给哪个 worker（1-{worker_count}） |
| title | -t | 是 | 任务目标，描述此 ticket 要完成什么 |
| context | -c | 是 | 技术上下文 + 整体 plan 摘要 |
| criteria | -k | 是 | 可验证的成功标准 |
| after | --after | 否 | 依赖的 ticket IDs，逗号分隔 |
| prompt | 最后位置参数 | 是 | 完整任务描述 |

## 字段写作要点

### title (-t) — 任务目标
描述此 ticket 的核心交付物。
- 好: "Implement user authentication API"、"Add SQLite persistence layer"
- 差: "Auth" / "Work on stuff"

### context (-c) — 技术上下文 + plan 概述
应包含：
- **整体 plan**：当前在做什么、目标是什么、此 ticket 在整体中的位置
- **技术栈**：语言、框架、版本
- **项目结构**：相关目录和文件
- **已有代码**：可复用的接口、类型、函数
- **约束**：性能要求、兼容性、不能用的依赖等
- 注意：不要包含工作目录路径，Worker 有自己的 worktree

### criteria (-k) — 成功标准
每条标准必须可验证：命令输出、文件存在、API 返回值。
- 好: "cargo test passes, POST /login returns 200 with JWT, invalid password returns 401"
- 差: "works correctly" / "good code quality"

### prompt (最后参数) — 完整实现指导
Worker 执行任务的全部信息，无需回来问问题。
- 具体要创建/修改哪些文件
- API 设计、数据结构、关键逻辑
- 边界情况处理

## 分解流程

### Step 1: 分析任务范围
- 理解用户需求的完整边界
- 识别独立的功能模块和文件组

### Step 2: 规划 ticket 划分
每个 ticket 应该：
- 一个 worker 能独立完成
- 操作的文件尽量不与其他 ticket 重叠
- 有明确的输入/输出边界
- 有可独立验证的 success criteria

### Step 3: 识别依赖关系 (DAG)
- ticket B 读取 ticket A 创建的文件 → B --after A
- ticket C 需要 A 和 B 的输出 → C --after A,B
- 完全无关的 ticket → 无 --after（并行执行）
- 原则：最小化依赖，最大化并行

### Step 4: 分配 worker
- 用 `legion-status` 查看当前 worker 状态
- 无依赖的 ticket 分配给不同 worker（并行）
- Worker 完成任务后会自动接新任务，无需等待

### Step 5: 批量 dispatch
列出所有 dispatch 命令，确认后逐个执行：
```
legion-dispatch 1 -t "..." -c "..." -k "..." "..."
legion-dispatch 2 -t "..." -c "..." -k "..." --after 1 "..."
legion-dispatch 3 -t "..." -c "..." -k "..." --after 1,2 "..."
```

## 分解原则

1. **文件边界优先**：不同 ticket 尽量操作不同文件/目录
2. **先基础后上层**：DB schema → API → UI → Tests
3. **最小依赖**：能并行就并行，只在必要时用 --after
4. **Context 要充分**：宁多勿少，Worker 看不到你的上下文
5. **Criteria 要可测**：每条标准必须能用命令验证
"#,
        worker_count = worker_count
    )
}
```

**Step 2: 运行 cargo check**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo check -p legion-tui`
Expected: PASS

**Step 3: Commit**

```bash
git add crates/legion-tui/src/claudemd.rs
git commit -m "feat: add split_tickets_command() for /split-tickets skill"
```

---

### Task 2: 添加 write_leader_commands() 函数

**Files:**
- Modify: `crates/legion-tui/src/claudemd.rs`

**Step 1: 在 split_tickets_command() 之后添加**

```rust
/// Write custom commands to the leader's worktree (.claude/commands/)
pub fn write_leader_commands(leader_worktree: &std::path::Path, worker_count: u16) -> Result<()> {
    let commands_dir = leader_worktree.join(".claude").join("commands");
    fs::create_dir_all(&commands_dir)?;

    let split_tickets_path = commands_dir.join("split-tickets.md");
    fs::write(&split_tickets_path, split_tickets_command(worker_count))?;

    Ok(())
}
```

**Step 2: 运行 cargo check**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo check -p legion-tui`
Expected: PASS

**Step 3: Commit**

```bash
git add crates/legion-tui/src/claudemd.rs
git commit -m "feat: add write_leader_commands() to generate .claude/commands/"
```

---

### Task 3: 修改 leader_instructions() 添加任务分解提示

**Files:**
- Modify: `crates/legion-tui/src/claudemd.rs`

**Step 1: 修改 Workflow 部分第 3 步**

将：
```
3. Split into tickets (one per worker)
```
改为：
```
3. Split into tickets — use `/split-tickets` command for task decomposition methodology
```

**Step 2: 在 Tools 部分之后、Rules 部分之前，新增两个部分**

```
## 任务分解提示

**核心规则：任何 plan 被确认后，不要直接开始执行，先询问是否需要分解为 tickets。**

在以下场景完成后，主动询问用户是否调用 `/split-tickets`：

1. **Plan mode 完成** — plan mode 中用户 approve plan 后
2. **Superpowers plan 完成** — brainstorming 或 writing-plans skill 生成 plan 后
3. **手动分析完成** — 你自行制定了多步骤实现方案后
4. **用户明确要求** — 用户说"分解任务"、"dispatch"、"split tickets"等
5. **任何其他形式的 plan 展示并确认后** — 不论 plan 来源如何，只要向用户展示了 plan 并获得确认，就提示

提示格式：
"Plan 已确定。是否使用 /split-tickets 分解为多个 ticket 并 dispatch 给 Workers？"

注意：
- 单步骤简单任务不需要分解，直接 dispatch 一个 ticket 即可
- 只有需要多个 worker 协作的多步骤任务才需要 /split-tickets
- **绝不跳过提示直接执行**

## Commands (可用命令)

- `/split-tickets` — 任务分解方法论，引导你将 plan 分解为 tickets 并批量 dispatch
```

**Step 3: 运行 cargo check + cargo test**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo check -p legion-tui && cargo test -p legion-tui`

**Step 4: 更新测试断言**

在 `leader_prompt_mentions_dispatch_format` 测试中添加：

```rust
assert!(prompt.contains("/split-tickets"));
assert!(prompt.contains("任务分解提示"));
assert!(prompt.contains("不要直接开始执行"));
```

**Step 5: Commit**

```bash
git add crates/legion-tui/src/claudemd.rs
git commit -m "feat: add split-tickets trigger prompts to leader system prompt"
```

---

### Task 4: 在 app.rs squad 启动时生成 command 文件

**Files:**
- Modify: `crates/legion-tui/src/app.rs`

**Step 1: 在生成 leader_prompt 之后添加**

找到 `let leader_prompt = crate::claudemd::leader_instructions(worker_count);` 行，在其后添加：

```rust
// Write /split-tickets command to leader worktree
if let Err(e) = crate::claudemd::write_leader_commands(&worktree_paths[0], worker_count) {
    tracing::warn!("Failed to write leader commands: {}", e);
}
```

**Step 2: 运行 cargo check**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo check -p legion-tui`
Expected: PASS

**Step 3: Commit**

```bash
git add crates/legion-tui/src/app.rs
git commit -m "feat: generate /split-tickets command on squad startup"
```

---

### Task 5: 添加测试

**Files:**
- Modify: `crates/legion-tui/src/claudemd.rs`

**Step 1: 添加测试**

```rust
#[test]
fn split_tickets_command_contains_key_elements() {
    let cmd = split_tickets_command(3);
    assert!(cmd.contains("legion-dispatch"));
    assert!(cmd.contains("--after"));
    assert!(cmd.contains("-t"));
    assert!(cmd.contains("-c"));
    assert!(cmd.contains("-k"));
    assert!(cmd.contains("3")); // worker count
    assert!(cmd.contains("context"));
    assert!(cmd.contains("criteria"));
    assert!(cmd.contains("DAG"));
    assert!(cmd.contains("prompt"));
}

#[test]
fn write_leader_commands_creates_files() {
    let dir = std::env::temp_dir().join("legion-test-commands");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    write_leader_commands(&dir, 3).unwrap();

    let path = dir.join(".claude/commands/split-tickets.md");
    assert!(path.exists());

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("Split Tickets"));
    assert!(content.contains("legion-dispatch"));

    let _ = std::fs::remove_dir_all(&dir);
}
```

**Step 2: 运行全量测试**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo test`
Expected: All PASS

**Step 3: Commit**

```bash
git add crates/legion-tui/src/claudemd.rs
git commit -m "test: add tests for split-tickets command generation"
```

---

### Task 6: 全量构建验证

**Step 1: cargo build --release**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo build --release`
Expected: PASS

**Step 2: cargo test**

Run: `cd /Users/rainlei/holiday/cc_router/legion && cargo test`
Expected: All PASS
