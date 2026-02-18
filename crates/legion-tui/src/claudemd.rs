//! Auto-generate CLAUDE.md files for Leader and Worker roles

use std::fs;
use std::path::PathBuf;

use anyhow::Result;

/// Generate the Leader's CLAUDE.md content
pub fn leader_instructions(worker_count: u16) -> String {
    format!(
        r#"# Squad Leader

You coordinate a team of {} autonomous Workers.

## MANDATORY DISPATCH FORMAT — READ THIS FIRST

Every `legion-dispatch` call MUST include ALL four parts. The command WILL FAIL without -t, -c, and -k:

```
legion-dispatch <worker_id> [--team <team_name>] -t "title" -c "context" -k "criteria" [--after N,M] "task description"
```

- `-t` — Short title (3-6 words): "Implement heart animation"
- `-c` — Context (language, dependencies, constraints): "Python 3, no external deps, terminal ANSI output"
- `-k` — Success criteria (testable conditions): "heart.py exists, python3 heart.py runs, uses math curve"
- `--after` — (Optional) Comma-separated ticket IDs this task depends on: "--after 1,3"
- `--team` — (Optional) Team template: tech_lead_team (default), fullstack_team, backend_team, qa_team, solo
- Last arg — Full task description with all implementation details

**IMPORTANT:** Do NOT include working directory paths in `-c`. Each Worker has its own dedicated worktree — they will create files in their current directory automatically.

Example:
```bash
legion-dispatch 1 -t "Implement heart animation" -c "Python 3, no external deps, terminal ANSI output" -k "heart.py exists, python3 heart.py shows animated heart, uses math-based curve" "Create heart.py with parametric heart curve animation using ANSI colors"
```

With dependency:
```bash
# With dependency:
legion-dispatch 2 -t "Add unit tests" -c "Python 3, pytest" -k "all tests pass" --after 1 "Write tests for heart.py animation"
```

With team:
```bash
legion-dispatch 1 --team fullstack_team -t "Implement heart animation" -c "Python 3, no external deps" -k "heart.py exists, python3 heart.py shows animated heart, uses math-based curve" "Create heart.py with parametric heart curve animation using ANSI colors"
```

## Workflow
1. Receive task from user
2. Analyze and create implementation plan
3. Split into tickets — use `/split-tickets` command for task decomposition methodology
4. Dispatch each ticket using the MANDATORY format above
5. Monitor with: `legion-check`
6. When all Workers complete, verify integration
7. Report results to user

## Task Dependencies

Use `--after` when tasks have file dependencies:
- Task B reads files Task A creates → `--after A`  (where A is the ticket ID)
- Task C depends on both A and B → `--after A,B`
- Independent tasks need no `--after` (they run in parallel)

Workers auto-receive code from completed dependencies via auto-merge.
Do NOT include `--after` for tasks that are truly independent.

## Tools
- `legion-dispatch` — Dispatch task (see format above)
- `legion-check` — View all Workers' status and results
- `legion-status` — Quick one-line status summary
- `legion-stop <id>` / `legion-stop all` — Emergency stop

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

## Rules
- Workers are AUTONOMOUS — do NOT expect replies. Use `legion-check` to poll.
- Workers retry automatically on failure (Ralph Loop).
- If a Worker errors after max retries, reassign or modify the task.
"#,
        worker_count
    )
}

/// Generate a Worker's system prompt: Agent Team lead with TechLeader/Engineer/QA
///
/// # Arguments
/// * `worker_id` - The worker's numeric ID
/// * `working_dir` - Optional working directory path for the worker
/// * `team_roles` - Optional slice of (role_id, role_name, prompt_template) tuples
pub fn worker_instructions(worker_id: u16, working_dir: Option<&str>, team_roles: Option<&[(String, String, String)]>) -> String {
    let wd_note = working_dir.map(|d| format!(
        "\n\n## Working Directory\n\nYour working directory is: `{}`\nAll files you create MUST be in this directory. Use relative paths (e.g., `./heart.py`) or this absolute path. NEVER write files to any other location.\n", d
    )).unwrap_or_default();

    // Base section: always included
    let mut prompt = format!(
        r#"# Worker {} — Autonomous Task Executor

You are Worker {}, running in headless non-interactive mode inside a Ralph Loop.{}
You receive a structured task with title, context, success criteria, and description.
Your job is to complete the task autonomously — no user interaction is possible.

## Execution Mode

This is a **headless SDK execution**. There is no terminal, no user input, no interactive prompts.
You must work completely autonomously from start to finish.
"#,
        worker_id, worker_id, wd_note
    );

    // Conditionally add Agent Team section
    if let Some(roles) = team_roles {
        if !roles.is_empty() {
            prompt.push_str("\n## Agent Team\n\n");
            prompt.push_str("You are the team lead of specialized agents. Your teammates are:\n\n");

            for (role_id, role_name, prompt_template) in roles {
                prompt.push_str(&format!(
                    "### {} (ID: {})\n{}\n\n",
                    role_name, role_id, prompt_template
                ));
            }

            prompt.push_str(
                r#"## Delegation Workflow

1. **Analyze the task**: Break down the work and identify which specialist should handle each part.
2. **Delegate**: Use the Task tool to assign work to the appropriate teammate:
   ```
   Task tool with subagent_type matching the role ID (e.g., "tech_lead", "frontend_engineer")
   ```
3. **Review**: When a teammate completes their work, review the output for quality and correctness.
4. **Verify all criteria**: Ensure every success criterion is met across all delegated work.
5. **Complete**: When ALL criteria pass, output `<promise>DONE</promise>` with a summary.

"#
            );
        }
    }

    // Solo workflow (only if no team or empty team)
    if team_roles.is_none() || team_roles.map_or(false, |r| r.is_empty()) {
        prompt.push_str(
            r#"
## Workflow

1. **Analyze the task**: Read the title, context, success criteria, and full description carefully.
2. **Plan**: Break the task into concrete, small implementation steps.
3. **Implement with TDD**:
   - For each step: write a failing test first, then implement the minimal code to pass it.
   - Run tests after each change to verify correctness.
   - Fix any failures before moving to the next step.
4. **Verify all criteria**: After implementation, check every success criterion listed in the task.
   - Run the full test suite.
   - Verify any specific behaviors mentioned in criteria.
5. **Complete**: When ALL criteria pass, output `<promise>DONE</promise>` with a brief summary.

"#
        );
    }

    // Footer: always included
    prompt.push_str(
        r#"## Output Format

When done, output exactly:
```
<promise>DONE</promise>
Summary: [brief description of what was implemented and verified]
```

## Error Handling

- If you encounter an error, debug it — read error messages, check logs, fix the code.
- If a test fails, analyze the failure and fix the implementation.
- Do NOT give up on first failure. Try alternative approaches.
- If the task is truly impossible (missing dependencies, wrong environment), explain why clearly.

## Critical Rules

- You are autonomous. Do NOT ask questions or wait for input.
- Follow TDD: test first, then implement.
- **STAY in the current working directory (pwd).** This is YOUR dedicated worktree. NEVER use `cd` to change to another directory. IGNORE any "working dir" path mentioned in the task context — it may point to the leader's directory, NOT yours. Your current directory is always correct. All file creation and commands must happen here.
- **When done, ALWAYS commit your work:**
  ```bash
  git add -A && git commit -m "feat: <short description of changes>"
  ```
- The `<promise>DONE</promise>` tag MUST appear in your output when the task is complete.
- Check EVERY success criterion before declaring done.
- Do NOT run long-running processes (servers, animations, infinite loops). Only create files and run quick tests/verifications.
- Keep your working directory clean — commit your changes when done.
"#
    );

    prompt
}

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

/// Write custom commands to the leader's worktree (.claude/commands/)
pub fn write_leader_commands(leader_worktree: &std::path::Path, worker_count: u16) -> Result<()> {
    let commands_dir = leader_worktree.join(".claude").join("commands");
    fs::create_dir_all(&commands_dir)?;

    let split_tickets_path = commands_dir.join("split-tickets.md");
    fs::write(&split_tickets_path, split_tickets_command(worker_count))?;

    Ok(())
}

/// Write CLAUDE.md files to a temp directory, return (leader_path, worker_paths)
pub fn write_squad_claude_md(worker_count: u16) -> Result<(PathBuf, Vec<PathBuf>)> {
    let dir = PathBuf::from("/tmp/legion/claudemd");
    fs::create_dir_all(&dir)?;

    let leader_path = dir.join("leader-CLAUDE.md");
    fs::write(&leader_path, leader_instructions(worker_count))?;

    let mut worker_paths = Vec::new();
    for i in 1..=worker_count {
        let path = dir.join(format!("worker-{}-CLAUDE.md", i));
        fs::write(&path, worker_instructions(i, None, None))?;
        worker_paths.push(path);
    }

    Ok((leader_path, worker_paths))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_prompt_contains_key_elements() {
        let prompt = worker_instructions(1, Some("/test/worktree"), None);
        assert!(prompt.contains("Worker 1"));
        assert!(prompt.contains("headless"));
        assert!(prompt.contains("TDD"));
        assert!(prompt.contains("<promise>DONE</promise>"));
        assert!(prompt.contains("success criteria") || prompt.contains("success criterion"));
        assert!(prompt.contains("autonomous"));
        assert!(prompt.contains("STAY in the current working directory"));
        assert!(prompt.contains("NEVER use `cd`"));
        assert!(prompt.contains("git add -A && git commit"));
        assert!(prompt.contains("/test/worktree"));
        assert!(prompt.contains("Your working directory is"));
    }

    #[test]
    fn leader_prompt_mentions_dispatch_format() {
        let prompt = leader_instructions(2);
        assert!(prompt.contains("MANDATORY DISPATCH FORMAT"));
        assert!(prompt.contains("legion-dispatch"));
        assert!(prompt.contains("legion-check"));
        assert!(prompt.contains("--after"));
        assert!(prompt.contains("/split-tickets"));
        assert!(prompt.contains("任务分解提示"));
        assert!(prompt.contains("不要直接开始执行"));
    }

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

    #[test]
    fn worker_prompt_with_team_roles() {
        let roles = vec![
            (
                "tech_lead".to_string(),
                "Technical Lead".to_string(),
                "You are the technical lead responsible for architecture and code reviews.".to_string(),
            ),
            (
                "frontend_engineer".to_string(),
                "Frontend Engineer".to_string(),
                "You specialize in React, TypeScript, and modern frontend development.".to_string(),
            ),
            (
                "qa_engineer".to_string(),
                "QA Engineer".to_string(),
                "You write comprehensive tests and ensure quality standards.".to_string(),
            ),
        ];

        let prompt = worker_instructions(1, None, Some(&roles));

        // Should contain Agent Team section
        assert!(prompt.contains("## Agent Team"));
        assert!(prompt.contains("team lead"));

        // Should contain all role names
        assert!(prompt.contains("Technical Lead"));
        assert!(prompt.contains("Frontend Engineer"));
        assert!(prompt.contains("QA Engineer"));

        // Should contain role IDs
        assert!(prompt.contains("tech_lead"));
        assert!(prompt.contains("frontend_engineer"));
        assert!(prompt.contains("qa_engineer"));

        // Should contain role templates
        assert!(prompt.contains("architecture and code reviews"));
        assert!(prompt.contains("React, TypeScript"));
        assert!(prompt.contains("comprehensive tests"));

        // Should contain delegation workflow
        assert!(prompt.contains("Delegation Workflow"));
        assert!(prompt.contains("Task tool"));

        // Should NOT contain solo TDD workflow
        assert!(!prompt.contains("Implement with TDD"));
    }

    #[test]
    fn worker_prompt_without_team_is_solo() {
        let prompt = worker_instructions(1, None, None);

        // Should contain solo TDD workflow
        assert!(prompt.contains("Implement with TDD"));
        assert!(prompt.contains("write a failing test first"));

        // Should NOT contain Agent Team section
        assert!(!prompt.contains("## Agent Team"));
        assert!(!prompt.contains("team lead"));
        assert!(!prompt.contains("Delegation Workflow"));
    }

    #[test]
    fn worker_prompt_with_empty_team_is_solo() {
        let empty_roles: Vec<(String, String, String)> = vec![];
        let prompt = worker_instructions(1, None, Some(&empty_roles));

        // Should contain solo TDD workflow
        assert!(prompt.contains("Implement with TDD"));

        // Should NOT contain Agent Team section
        assert!(!prompt.contains("## Agent Team"));
    }
}
