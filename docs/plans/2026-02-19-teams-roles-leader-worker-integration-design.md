# Teams/Roles Leader-Worker Integration Design

**Goal:** Fully integrate Teams and Roles into the Leader and Worker prompt generation pipeline, so that:
1. Leader dynamically knows all available teams and their roles (no hardcoded list)
2. Workers receive a complete collaboration framework when assigned a team
3. Team-level orchestration guidance (team_prompt) is user-editable in the TUI

**Approach:** Add `team_prompt` field to Team data model, rewrite `leader_instructions()` and `worker_instructions()` in claudemd.rs, update TUI form.

**Key Decisions:**
- Leader prompt includes dynamic team directory loaded from DB at startup
- Worker Agent Team section includes: team objective, role hierarchy, mixed communication protocol (sequential + parallel), entropy reduction rules
- No human-in-the-loop triggers in worker (workers should complete autonomously)
- Team data model gains `team_prompt TEXT` field for team-level collaboration guide
- TeamForm gains a 3rd text field for team_prompt editing (multi-line)

---

## 1. DB Layer: Team team_prompt Field

**Change:** Add `team_prompt TEXT DEFAULT ''` column to teams table.

**Migration:** `ALTER TABLE teams ADD COLUMN team_prompt TEXT DEFAULT ''`

**Seed data:** Populate builtin teams with default team_prompt values:

- `tech_lead_team`: "This team follows a structured development workflow: 1) Tech Lead analyzes requirements and designs architecture 2) Engineer implements with strict TDD 3) QA validates all acceptance criteria. Communication: Sequential delegation by default. Use parallel only for independent review tasks."

- `fullstack_team`: "Architecture-driven team: 1) Architect designs system structure and evaluates trade-offs 2) Engineer implements following the architecture 3) QA tests comprehensively. Communication: Architect reviews before Engineer starts. Parallel QA alongside implementation for independent test writing."

- `backend_team`: "Focused backend pair: 1) Tech Lead plans and reviews 2) Engineer implements with TDD. Communication: Sequential — plan first, then implement."

- `qa_team`: "Quality-focused pair: 1) Engineer implements features 2) QA writes independent tests and validates. Communication: Parallel — QA can write test specs while Engineer implements."

- `solo`: "" (empty — solo mode has no team collaboration)

**Files:** `crates/legion-db/src/schema.rs`, `crates/legion-db/src/repo.rs`

## 2. Leader Prompt: Dynamic Team Directory

**Change:** `leader_instructions(worker_count: u16)` → `leader_instructions(worker_count: u16, teams: &[(String, String, Vec<String>)])`

Each tuple is `(team_id, team_name, role_names)`.

**Replace** hardcoded line 27 (`--team — (Optional) Team template: tech_lead_team, fullstack_team, ...`) with dynamically generated team directory:

```
## Available Teams

| Team ID | Name | Roles |
|---------|------|-------|
| tech_lead_team | Tech Lead Team | Tech Lead, Engineer, QA |
| fullstack_team | Fullstack Team | Architect, Engineer, QA |
| solo | Solo | (no roles, TDD mode) |

Use `--team <team_id>` to assign a team to a ticket.
Default: tech_lead_team (if not specified)
```

**Caller:** App must load teams from DB when generating leader CLAUDE.md (in `write_squad_claude_md` and wherever leader prompt is assembled).

**Files:** `crates/legion-tui/src/claudemd.rs`, `crates/legion-tui/src/app.rs` or `crates/legion-tui/src/lib.rs`

## 3. Worker Prompt: Complete Collaboration Framework

**Change:** Rewrite the Agent Team section in `worker_instructions()`.

**New signature:** `worker_instructions(worker_id, working_dir, team_roles, team_prompt)`

Where `team_prompt: Option<&str>` is the team-level collaboration guide.

**Generated content when team_roles is non-empty:**

```markdown
## Agent Team

### Team Objective
{team_prompt}

### Your Teammates

#### {role_name} (ID: {role_id})
{prompt_template}

[... for each role ...]

### Communication Protocol

You have two delegation modes. Choose based on task structure:

**Sequential (default):** Delegate to one teammate at a time via the Task tool. Wait for their result, review it, then decide the next step.
- Use when: tasks have dependencies, quality review needed between steps
- Example flow: Tech Lead designs → Engineer implements → QA validates

**Parallel (broadcast):** Delegate to multiple teammates simultaneously via multiple Task tool calls in one message.
- Use when: tasks are independent, need diverse perspectives, or time-sensitive
- Example: Multiple engineers implement different components; QA writes test specs while Engineer codes

### Entropy Reduction Rules

1. Each role focuses ONLY on their stated domain — do not cross-assign responsibilities
2. When reviewing teammate output, provide SPECIFIC actionable feedback (not "looks good")
3. If roles produce conflicting results, YOU (team lead) make the final decision based on success criteria
4. Do not re-delegate the same work without concrete changes to the instructions
5. Prefer fewer, more targeted delegations over many vague ones

### Completion
When ALL success criteria are met, output `<promise>DONE</promise>` with a summary of what each role contributed.
```

**Solo mode (no change):** When team_roles is None or empty, keep existing TDD workflow.

**Files:** `crates/legion-tui/src/claudemd.rs`

## 4. TUI: TeamForm team_prompt Field

**Change:** TeamForm adds team_prompt as the 3rd text field (before role selection).

- `team_form_fields` changes from `[String; 2]` to `[String; 3]` → `[name, description, team_prompt]`
- Focus zones become: 0=name, 1=description, 2=team_prompt (multi-line), 3=role selection
- team_prompt field renders with word-wrap (reuse existing `wrap_text()`)
- Cursor support same as other text fields
- Tab/Shift+Tab/Up/Down cycles through all 4 zones

**Display:** team_prompt area shows label "Team Prompt:" with gray text preview when unfocused, editable when focused.

**Files:** `crates/legion-tui/src/app.rs`, `crates/legion-tui/src/input.rs`, `crates/legion-tui/src/ui.rs`

## 5. Data Flow Summary

```
TUI (team_prompt editable) → DB (team_prompt column)
                                    ↓
Leader start → load teams from DB → leader_instructions(count, teams)
                                    ↓
                            Leader CLAUDE.md includes dynamic team directory
                                    ↓
Leader dispatches → --team custom_team → API → TaskTicket(team_mode)
                                    ↓
Worker takes ticket → resolve_team_roles(team_id) → DB query
                    → get_team(team_id).team_prompt → DB query
                                    ↓
worker_instructions(id, wd, team_roles, team_prompt)
                                    ↓
Worker CLAUDE.md includes full collaboration framework
```
