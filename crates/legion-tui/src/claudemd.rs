//! Auto-generate CLAUDE.md files for Leader and Worker roles

use std::fs;
use std::path::PathBuf;

use anyhow::Result;

/// Generate the Leader's CLAUDE.md content
pub fn leader_instructions(worker_count: u16) -> String {
    format!(
        r#"# Squad Leader

You coordinate a team of {} autonomous Workers.

## ⚠️ MANDATORY DISPATCH FORMAT — READ THIS FIRST

Every `legion-dispatch` call MUST include ALL four parts. The command WILL FAIL without -t, -c, and -k:

```
legion-dispatch <worker_id> [--team <team_name>] -t "title" -c "context" -k "criteria" "task description"
```

- `-t` — Short title (3-6 words): "Implement heart animation"
- `-c` — Context (working dir, language, files): "Python 3, working dir: ./scripts, no deps"
- `-k` — Success criteria (testable conditions): "heart.py exists, python3 heart.py runs, uses math curve"
- `--team` — (Optional) Team template: tech_lead_team (default), fullstack_team, backend_team, qa_team, solo
- Last arg — Full task description with all implementation details

Example:
```bash
legion-dispatch 1 --team fullstack_team -t "Implement heart animation" -c "Working dir: ./scripts, Python 3, no external deps" -k "heart.py exists, python3 heart.py shows animated heart, uses math-based curve" "Create heart.py with parametric heart curve animation using ANSI colors"
```

## Workflow
1. Receive task from user
2. Analyze and create implementation plan
3. Split into tickets (one per worker)
4. Dispatch each ticket using the MANDATORY format above
5. Monitor with: `legion-check`
6. When all Workers complete, verify integration
7. Report results to user

## Tools
- `legion-dispatch` — Dispatch task (see format above)
- `legion-check` — View all Workers' status and results
- `legion-status` — Quick one-line status summary
- `legion-stop <id>` / `legion-stop all` — Emergency stop

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
/// * `team_roles` - Optional slice of (role_id, role_name, prompt_template) tuples
pub fn worker_instructions(worker_id: u16, team_roles: Option<&[(String, String, String)]>) -> String {
    // Base section: always included
    let mut prompt = format!(
        r#"# Worker {} — Autonomous Task Executor

You are Worker {}, running in headless non-interactive mode inside a Ralph Loop.
You receive a structured task with title, context, success criteria, and description.
Your job is to complete the task autonomously — no user interaction is possible.

## Execution Mode

This is a **headless SDK execution**. There is no terminal, no user input, no interactive prompts.
You must work completely autonomously from start to finish.
"#,
        worker_id, worker_id
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
- The `<promise>DONE</promise>` tag MUST appear in your output when the task is complete.
- Check EVERY success criterion before declaring done.
- Keep your working directory clean — commit your changes when done.
"#
    );

    prompt
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
        fs::write(&path, worker_instructions(i, None))?;
        worker_paths.push(path);
    }

    Ok((leader_path, worker_paths))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_prompt_contains_key_elements() {
        let prompt = worker_instructions(1, None);
        assert!(prompt.contains("Worker 1"));
        assert!(prompt.contains("headless"));
        assert!(prompt.contains("TDD"));
        assert!(prompt.contains("<promise>DONE</promise>"));
        assert!(prompt.contains("success criteria") || prompt.contains("success criterion"));
        assert!(prompt.contains("autonomous"));
    }

    #[test]
    fn leader_prompt_mentions_dispatch_format() {
        let prompt = leader_instructions(2);
        assert!(prompt.contains("MANDATORY DISPATCH FORMAT"));
        assert!(prompt.contains("legion-dispatch"));
        assert!(prompt.contains("legion-check"));
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

        let prompt = worker_instructions(1, Some(&roles));

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
        let prompt = worker_instructions(1, None);

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
        let prompt = worker_instructions(1, Some(&empty_roles));

        // Should contain solo TDD workflow
        assert!(prompt.contains("Implement with TDD"));

        // Should NOT contain Agent Team section
        assert!(!prompt.contains("## Agent Team"));
    }
}
