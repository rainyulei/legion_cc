# Worker Agent Team Mode Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Worker 收到任务后自动创建 Agent Team（TechLeader/Engineer/QA），在 Ralph Loop 保障下迭代完成任务。

**Architecture:** 两处改动：1) `pty.rs` 为 Worker pane 设置 `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1` 环境变量启用 Agent Teams；2) `claudemd.rs` 重写 `worker_instructions()` 为完整的 Agent Team 编排 prompt，包含三个 teammate 角色定义、顺序流水线、Ralph Loop `<promise>` 退出机制和 `legion-report` 收尾。

**Tech Stack:** Rust, Claude Code Agent Teams (experimental), Ralph Loop

---

### Task 1: Worker PTY 启用 Agent Teams 环境变量

**Files:**
- Modify: `crates/legion-tui/src/pty.rs:66-71`

**Step 1: 在 `worker_id` 环境变量设置后，添加 Agent Teams 环境变量**

在 `pty.rs` 的 `spawn()` 函数中，找到这段代码（第 66-71 行）：

```rust
        if let Some(wid) = worker_id {
            cmd.env("LEGION_WORKER_ID", wid.to_string());
        }
        if let Some(op) = orchestrate_port {
            cmd.env("LEGION_ORCHESTRATE_PORT", op.to_string());
        }
```

在 `LEGION_WORKER_ID` 设置之后、`orchestrate_port` 设置之前，添加 Agent Teams 环境变量：

```rust
        if let Some(wid) = worker_id {
            cmd.env("LEGION_WORKER_ID", wid.to_string());
            // Enable Claude Code Agent Teams for Worker panes
            cmd.env("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "1");
        }
        if let Some(op) = orchestrate_port {
            cmd.env("LEGION_ORCHESTRATE_PORT", op.to_string());
        }
```

关键：只有 `worker_id.is_some()` 的 pane（即 Worker）才启用 Agent Teams。Leader 不受影响。

**Step 2: 编译验证**

Run: `cargo build --bin legion 2>&1`
Expected: 编译通过，无新 error

**Step 3: Commit**

```bash
git add crates/legion-tui/src/pty.rs
git commit -m "feat: enable Agent Teams env var for Worker PTY panes"
```

---

### Task 2: 重写 worker_instructions() — Agent Team 编排 prompt

**Files:**
- Modify: `crates/legion-tui/src/claudemd.rs:44-66`

**Step 1: 替换 `worker_instructions()` 函数**

将当前的 `worker_instructions()` 函数（第 44-66 行）完整替换为以下内容：

```rust
/// Generate a Worker's system prompt: Agent Team lead with TechLeader/Engineer/QA
pub fn worker_instructions(worker_id: u16) -> String {
    format!(
        r#"# Worker {} — Agent Team Lead

You are Worker {}, an autonomous Agent Team lead running inside a Ralph Loop.
When you receive a task, you MUST create an agent team and coordinate it to completion.

## Step 1: Create Agent Team

Immediately create an agent team with exactly 3 teammates. Use delegate mode (Shift+Tab) so you only coordinate, never implement directly.

Spawn teammates with these exact prompts:

**TechLeader teammate:**
"You are TechLeader. Your responsibilities:
1. Read the ticket carefully. Understand every requirement and success criterion.
2. Break the task into the smallest possible implementable and testable steps.
3. Create a task list with clear descriptions for each step.
4. After ALL tasks are completed by Engineer and verified by QA, do a final review:
   - Check every success criterion from the original ticket
   - Verify code quality and test coverage
   - If PASSED: message the lead with 'REVIEW PASSED' and a brief summary
   - If FAILED: create fix tasks and assign back to Engineer
Do NOT implement code yourself. Only plan, review, and coordinate."

**Engineer teammate:**
"You are Engineer. Your responsibilities:
1. Claim tasks from the task list created by TechLeader, in order.
2. Implement the code following TDD: write failing test first, then implement.
3. Run tests to verify your implementation works.
4. When QA reports issues, fix them immediately.
5. After fixing, notify QA to re-test.
Do NOT skip tests. Do NOT move to next task until current one passes QA."

**QA teammate:**
"You are QA. Your responsibilities:
1. After Engineer completes each task, test it thoroughly.
2. Run all relevant tests. Check edge cases.
3. If tests PASS: mark the task as completed.
4. If tests FAIL: message Engineer with specific failure details and expected behavior.
5. After Engineer fixes, re-test. Loop until passing.
Do NOT implement fixes yourself. Only test and report."

## Step 2: Coordinate Pipeline

After spawning teammates, the workflow is:
1. TechLeader analyzes ticket → creates task list
2. Engineer claims tasks sequentially → implements with TDD
3. QA tests after each Engineer completion → loops with Engineer on failures
4. After all tasks done, TechLeader does final review against success criteria
5. If TechLeader reports REVIEW PASSED → proceed to Step 3

Wait for teammates to complete. Do NOT implement anything yourself.

## Step 3: Complete and Report

When TechLeader reports "REVIEW PASSED":
1. Ask TechLeader teammate to shut down
2. Ask Engineer teammate to shut down
3. Ask QA teammate to shut down
4. Clean up the team
5. Output exactly: <promise>DONE</promise>
6. Run: `legion-report done "SUMMARY_FROM_TECHLEADER"`

Replace SUMMARY_FROM_TECHLEADER with the actual summary from TechLeader's review.

## Error Handling

- If a teammate stops unexpectedly, spawn a replacement with the same role prompt.
- If the task seems impossible after multiple attempts, run: `legion-report error "description of what went wrong"`
- Do NOT give up on first failure. The Ralph Loop will retry if needed.

## Critical Rules

- ALWAYS use delegate mode. You are the coordinator, not an implementer.
- ALWAYS spawn exactly 3 teammates: TechLeader, Engineer, QA.
- NEVER skip the TechLeader final review.
- NEVER report done without TechLeader confirmation.
- The <promise>DONE</promise> tag MUST appear in your output to exit the Ralph Loop.
"#,
        worker_id, worker_id
    )
}
```

**Step 2: 编译验证**

Run: `cargo build --bin legion 2>&1`
Expected: 编译通过，无新 error

**Step 3: Commit**

```bash
git add crates/legion-tui/src/claudemd.rs
git commit -m "feat: rewrite worker prompt for Agent Team mode with TechLeader/Engineer/QA pipeline"
```

---

### Task 3: 更新 leader_instructions() 反映 Worker 新行为

**Files:**
- Modify: `crates/legion-tui/src/claudemd.rs:9-42`

**Step 1: 更新 Leader prompt 中关于 Worker 的描述**

将 `leader_instructions()` 中的 `## Important` 部分更新，让 Leader 知道 Worker 现在内部有 Agent Team：

找到这段（约第 34-39 行）：
```rust
## Important
- Workers are AUTONOMOUS. Do NOT expect replies from them.
- Each Worker will execute independently using TDD.
- Use `legion-check` to poll for completion — it does not interrupt your work.
- If a Worker reports an error, decide whether to reassign, modify, or abort.
```

替换为：
```rust
## Important
- Workers are AUTONOMOUS. Each Worker runs an internal Agent Team (TechLeader + Engineer + QA).
- Workers use Ralph Loop for reliability — they will retry automatically on failure.
- Do NOT expect replies from Workers. Use `legion-check` to poll for completion.
- Include clear **success criteria** in each ticket — the Worker's TechLeader uses them for final review.
- If a Worker reports an error after multiple retries, decide whether to reassign, modify, or abort.
```

**Step 2: 编译验证**

Run: `cargo build --bin legion 2>&1`
Expected: 编译通过

**Step 3: Commit**

```bash
git add crates/legion-tui/src/claudemd.rs
git commit -m "feat: update leader prompt to describe Worker Agent Team behavior"
```

---

### Task 4: 全量编译验证

**Step 1: 全 workspace 编译**

Run: `cargo build 2>&1`
Expected: 全部编译通过

**Step 2: 运行已有测试**

Run: `cargo test 2>&1`
Expected: 所有测试通过（proxy server tests、transform tests 等）

**Step 3: 检查 prompt 内容正确性**

手动验证 `worker_instructions(1)` 的输出格式：

Run: `cargo test -p legion-tui 2>&1` (如果有测试)

如果没有 legion-tui 的测试，用以下方式验证：在 `claudemd.rs` 末尾临时加一个测试：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_prompt_contains_agent_team() {
        let prompt = worker_instructions(1);
        assert!(prompt.contains("Agent Team Lead"));
        assert!(prompt.contains("TechLeader teammate"));
        assert!(prompt.contains("Engineer teammate"));
        assert!(prompt.contains("QA teammate"));
        assert!(prompt.contains("<promise>DONE</promise>"));
        assert!(prompt.contains("legion-report done"));
        assert!(prompt.contains("delegate mode"));
    }

    #[test]
    fn leader_prompt_mentions_agent_team() {
        let prompt = leader_instructions(2);
        assert!(prompt.contains("Agent Team"));
        assert!(prompt.contains("TechLeader"));
        assert!(prompt.contains("success criteria"));
    }
}
```

Run: `cargo test -p legion-tui 2>&1`
Expected: 2 tests passed

**Step 4: Commit (含测试)**

```bash
git add crates/legion-tui/src/claudemd.rs
git commit -m "test: add prompt content tests for worker and leader instructions"
```
