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
4. Dispatch with: `legion-dispatch <worker_id> "ticket content"`
5. Include in each ticket:
   - Clear task description
   - Test success criteria
   - Relevant file paths and context
6. Monitor with: `legion-check`
7. When all Workers complete, verify integration
8. Report results to user

## Tools
- `legion-dispatch <id> "ticket"` — Send task to Worker
- `legion-check` — View all Workers' status and results
- `legion-status` — Quick one-line status summary
- `legion-stop <id>` / `legion-stop all` — Emergency stop

## Important
- Workers are AUTONOMOUS. Do NOT expect replies from them.
- Each Worker will execute independently using TDD.
- Use `legion-check` to poll for completion — it does not interrupt your work.
- If a Worker reports an error, decide whether to reassign, modify, or abort.
"#,
        worker_count
    )
}

/// Generate a Worker's CLAUDE.md content
pub fn worker_instructions(worker_id: u16) -> String {
    format!(
        r#"# Worker {} — Autonomous Task Executor

You are an autonomous worker. Execute the assigned task using TDD:

1. Read the task description carefully
2. Implement the code
3. Write tests matching the success criteria
4. Run tests until all pass
5. When complete, run: `legion-report done "brief summary of what was done"`
6. If you encounter an unrecoverable error, run: `legion-report error "description of the error"`

## Rules
- Do NOT ask for clarification. Make reasonable decisions.
- Do NOT wait for instructions. Execute immediately.
- Focus ONLY on your assigned task.
- Test thoroughly before reporting done.
"#,
        worker_id
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
