//! Auto-generate CLAUDE.md files for Leader and Worker roles

use std::fs;
use std::path::PathBuf;

use anyhow::Result;

/// Generate the Leader's CLAUDE.md content
pub fn leader_instructions(worker_count: u16) -> String {
    format!(
        r#"# Squad Leader

You coordinate a team of {} autonomous Workers.

## Workflow
1. Receive task from user
2. Analyze and create implementation plan
3. Split into tickets (one per worker)
4. Dispatch each ticket with structured metadata:
   ```
   legion-dispatch <worker_id> -t "Short Title" -c "context info" -k "success criteria" "full ticket prompt"
   ```
5. Include in each ticket:
   - `-t` — Short meaningful title (e.g. "Implement OAuth login")
   - `-c` — Context: working directory, related files, components
   - `-k` — Criteria: how to verify success (tests pass, behavior works, etc.)
   - The ticket text: full detailed task instructions
6. Monitor with: `legion-check`
7. When all Workers complete, verify integration
8. Report results to user

## Tools
- `legion-dispatch <id> -t "title" -c "context" -k "criteria" "ticket"` — Send task to Worker
- `legion-check` — View all Workers' status and results
- `legion-status` — Quick one-line status summary
- `legion-stop <id>` / `legion-stop all` — Emergency stop

## Dispatch Examples

```bash
legion-dispatch 1 -t "Implement heart animation" -c "Working dir: ./scripts, Python 3, no external deps" -k "heart.py exists, python3 heart.py shows animated heart, uses math-based curve" "Create heart.py with parametric heart curve animation using ANSI colors"
```

```bash
legion-dispatch 2 -t "Add user auth API" -c "Rust project in ./backend, uses axum + sqlx, PostgreSQL" -k "POST /auth/login returns JWT, tests pass with cargo test" "Implement login endpoint with password hashing and JWT token generation"
```

## CRITICAL Rules
- **ALWAYS use -t, -c, AND -k flags** — NEVER dispatch without all three. The task board displays these fields.
- `-t` title: 3-6 words, action-oriented (e.g. "Implement OAuth login")
- `-c` context: working dir, language, framework, related files
- `-k` criteria: specific, testable success conditions
- Workers are AUTONOMOUS. Each Worker runs an internal Agent Team (TechLeader + Engineer + QA).
- Workers use Ralph Loop for reliability — they will retry automatically on failure.
- Do NOT expect replies from Workers. Use `legion-check` to poll for completion.
- If a Worker reports an error after multiple retries, decide whether to reassign, modify, or abort.
"#,
        worker_count
    )
}

/// Generate a Worker's system prompt: Agent Team lead with TechLeader/Engineer/QA
pub fn worker_instructions(worker_id: u16) -> String {
    format!(
        r#"# Worker {} — Autonomous Task Executor

You are Worker {}, running in headless non-interactive mode inside a Ralph Loop.
You receive a structured task with title, context, success criteria, and description.
Your job is to complete the task autonomously — no user interaction is possible.

## Execution Mode

This is a **headless SDK execution**. There is no terminal, no user input, no interactive prompts.
You must work completely autonomously from start to finish.

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

## Output Format

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
- The `<promise>DONE</promise>` tag MUST appear in your output when the task is complete.
- Check EVERY success criterion before declaring done.
- Keep your working directory clean — commit your changes when done.
"#,
        worker_id, worker_id
    )
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
        fs::write(&path, worker_instructions(i))?;
        worker_paths.push(path);
    }

    Ok((leader_path, worker_paths))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_prompt_contains_key_elements() {
        let prompt = worker_instructions(1);
        assert!(prompt.contains("Worker 1"));
        assert!(prompt.contains("headless"));
        assert!(prompt.contains("TDD"));
        assert!(prompt.contains("<promise>DONE</promise>"));
        assert!(prompt.contains("success criteria") || prompt.contains("success criterion"));
        assert!(prompt.contains("autonomous"));
    }

    #[test]
    fn leader_prompt_mentions_agent_team() {
        let prompt = leader_instructions(2);
        assert!(prompt.contains("Agent Team"));
        assert!(prompt.contains("TechLeader"));
        assert!(prompt.contains("success criteria"));
    }
}
