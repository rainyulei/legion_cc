use rusqlite::Connection;
use anyhow::Result;

/// Global DB schema: providers, roles, teams, pane_configs
pub const GLOBAL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS providers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    api_key TEXT,
    api_format TEXT DEFAULT 'anthropic',
    models TEXT,
    is_default INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS pane_configs (
    pane_label TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    model TEXT,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS roles (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    prompt_template TEXT NOT NULL,
    is_builtin INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS teams (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL,
    role_ids TEXT NOT NULL,
    is_builtin INTEGER DEFAULT 0,
    created_at INTEGER NOT NULL
);
"#;

/// Project DB schema: sessions, tickets, workers
pub const PROJECT_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    name TEXT,
    project_path TEXT,
    claude_session_file TEXT,
    provider_id TEXT,
    created_at INTEGER NOT NULL,
    last_active_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS pending_questions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    worker_id TEXT NOT NULL,
    risk_level TEXT NOT NULL,
    content TEXT NOT NULL,
    context TEXT,
    status TEXT DEFAULT 'pending',
    answer TEXT,
    created_at INTEGER NOT NULL,
    answered_at INTEGER
);

CREATE TABLE IF NOT EXISTS workers (
    id TEXT PRIMARY KEY,
    role TEXT NOT NULL,
    status TEXT DEFAULT 'idle',
    current_task TEXT,
    provider_id TEXT,
    session_id TEXT,
    proxy_port INTEGER,
    pid INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS squad_sessions (
    name TEXT PRIMARY KEY,
    project_path TEXT NOT NULL,
    worker_count INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    created_at INTEGER NOT NULL,
    completed_at INTEGER,
    is_default INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS tickets (
    id INTEGER PRIMARY KEY,
    session_name TEXT NOT NULL,
    title TEXT NOT NULL,
    prompt TEXT NOT NULL,
    context TEXT,
    criteria TEXT,
    status TEXT NOT NULL DEFAULT 'queued',
    assigned_worker INTEGER,
    team_mode TEXT NOT NULL DEFAULT 'tech_lead_team',
    iteration INTEGER NOT NULL DEFAULT 0,
    max_iterations INTEGER NOT NULL DEFAULT 5,
    feedback TEXT,
    summary TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    origin_session TEXT
);

CREATE TABLE IF NOT EXISTS ticket_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ticket_id INTEGER NOT NULL,
    session_name TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS ticket_diffs (
    ticket_id INTEGER PRIMARY KEY,
    session_name TEXT NOT NULL,
    diff_content TEXT NOT NULL,
    file_summary TEXT NOT NULL,
    cached_at INTEGER NOT NULL
);
"#;

/// Initialize global DB (providers, roles, teams, pane_configs + seed data)
pub fn init_global_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(GLOBAL_SCHEMA)?;
    // Migrations
    let _ = conn.execute("ALTER TABLE teams ADD COLUMN team_prompt TEXT DEFAULT ''", []);

    // Seed roles
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let _ = conn.execute("INSERT OR IGNORE INTO roles (id, name, description, prompt_template, is_builtin, created_at) VALUES (?1, ?2, ?3, ?4, 1, ?5)",
        ["tech_lead", "Tech Lead", "Technical lead responsible for architecture and planning",
         "You are the Tech Lead. Your approach: 1) **Technical Analysis**: Analyze requirements, identify dependencies, review existing code patterns, design the approach. 2) **Plan Alignment**: Review the Structure Plan (if provided) to ensure file paths match, import conventions are followed, new code integrates with existing modules. 3) **Produce Execution Guide**: Before delegating to Engineers, write a brief guide: exact file paths, import conventions, integration points, testing approach. 4) Review all code for correctness and edge cases. Focus on planning before implementation. Never deviate from the Structure Plan's file path conventions.", &now.to_string()]);

    let _ = conn.execute("INSERT OR IGNORE INTO roles (id, name, description, prompt_template, is_builtin, created_at) VALUES (?1, ?2, ?3, ?4, 1, ?5)",
        ["engineer", "Engineer", "Software engineer responsible for implementation",
         "You are the Engineer. Your approach: 1) Follow strict TDD - write a failing test first. 2) Implement the minimal code to pass the test. 3) Refactor for clarity. 4) Repeat until all requirements are met. Never skip writing tests.", &now.to_string()]);

    let _ = conn.execute("INSERT OR IGNORE INTO roles (id, name, description, prompt_template, is_builtin, created_at) VALUES (?1, ?2, ?3, ?4, 1, ?5)",
        ["qa", "QA Engineer", "Quality assurance engineer responsible for testing",
         "You are the QA Engineer. Your approach: 1) Read every acceptance criterion carefully. 2) Write tests for each criterion including edge cases and error paths. 3) Run the full test suite. 4) Report any failures with clear reproduction steps. Be thorough and skeptical.", &now.to_string()]);

    let _ = conn.execute("INSERT OR IGNORE INTO roles (id, name, description, prompt_template, is_builtin, created_at) VALUES (?1, ?2, ?3, ?4, 1, ?5)",
        ["pm", "Product Manager", "Product manager responsible for requirements",
         "You are the Product Manager. Your approach: 1) Evaluate requirements for completeness and clarity. 2) Identify missing edge cases from the user perspective. 3) Define clear acceptance criteria. 4) Validate the final result matches user intent.", &now.to_string()]);

    let _ = conn.execute("INSERT OR IGNORE INTO roles (id, name, description, prompt_template, is_builtin, created_at) VALUES (?1, ?2, ?3, ?4, 1, ?5)",
        ["architect", "Architect", "System architect responsible for system design",
         "You are the Architect. Your approach: 1) Evaluate the system design and technology choices. 2) Consider scalability, performance, and security implications. 3) Identify potential bottlenecks or vulnerabilities. 4) Document architectural decisions and trade-offs.", &now.to_string()]);

    let _ = conn.execute("INSERT OR IGNORE INTO roles (id, name, description, prompt_template, is_builtin, created_at) VALUES (?1, ?2, ?3, ?4, 1, ?5)",
        ["devops", "DevOps Engineer", "DevOps engineer responsible for deployment",
         "You are the DevOps Engineer. Your approach: 1) Set up reproducible build and deployment processes. 2) Write Dockerfiles or deployment scripts as needed. 3) Configure CI/CD pipelines. 4) Ensure monitoring and logging are in place.", &now.to_string()]);

    // Seed teams
    let _ = conn.execute("INSERT OR IGNORE INTO teams (id, name, description, role_ids, is_builtin, created_at) VALUES (?1, ?2, ?3, ?4, 1, ?5)",
        ["tech_lead_team", "Tech Lead Team", "Team with tech lead, engineer, and QA", r#"["tech_lead","engineer","qa"]"#, &now.to_string()]);

    let _ = conn.execute("INSERT OR IGNORE INTO teams (id, name, description, role_ids, is_builtin, created_at) VALUES (?1, ?2, ?3, ?4, 1, ?5)",
        ["fullstack_team", "Fullstack Team", "Team with architect, engineer, and QA", r#"["architect","engineer","qa"]"#, &now.to_string()]);

    let _ = conn.execute("INSERT OR IGNORE INTO teams (id, name, description, role_ids, is_builtin, created_at) VALUES (?1, ?2, ?3, ?4, 1, ?5)",
        ["backend_team", "Backend Team", "Team with tech lead and engineer", r#"["tech_lead","engineer"]"#, &now.to_string()]);

    let _ = conn.execute("INSERT OR IGNORE INTO teams (id, name, description, role_ids, is_builtin, created_at) VALUES (?1, ?2, ?3, ?4, 1, ?5)",
        ["qa_team", "QA Team", "Team with QA and engineer", r#"["qa","engineer"]"#, &now.to_string()]);

    let _ = conn.execute("INSERT OR IGNORE INTO teams (id, name, description, role_ids, is_builtin, created_at) VALUES (?1, ?2, ?3, ?4, 1, ?5)",
        ["solo", "Solo", "Solo mode with no team", "[]", &now.to_string()]);

    // Migrate existing tech_lead role prompt to include plan alignment
    let _ = conn.execute("UPDATE roles SET prompt_template = ?1 WHERE id = 'tech_lead' AND prompt_template NOT LIKE '%Structure Plan%'",
        ["You are the Tech Lead. Your approach: 1) **Technical Analysis**: Analyze requirements, identify dependencies, review existing code patterns, design the approach. 2) **Plan Alignment**: Review the Structure Plan (if provided) to ensure file paths match, import conventions are followed, new code integrates with existing modules. 3) **Produce Execution Guide**: Before delegating to Engineers, write a brief guide: exact file paths, import conventions, integration points, testing approach. 4) Review all code for correctness and edge cases. Focus on planning before implementation. Never deviate from the Structure Plan's file path conventions."]);

    let _ = conn.execute("UPDATE teams SET team_prompt = ?1 WHERE id = 'tech_lead_team' AND team_prompt = ''",
        ["This team follows a structured development workflow: 1) Tech Lead analyzes requirements and designs architecture 2) Engineer implements with strict TDD 3) QA validates all acceptance criteria. Communication: Sequential delegation by default. Use parallel only for independent review tasks."]);

    let _ = conn.execute("UPDATE teams SET team_prompt = ?1 WHERE id = 'fullstack_team' AND team_prompt = ''",
        ["Architecture-driven team: 1) Architect designs system structure and evaluates trade-offs 2) Engineer implements following the architecture 3) QA tests comprehensively. Communication: Architect reviews before Engineer starts. Parallel QA alongside implementation for independent test writing."]);

    let _ = conn.execute("UPDATE teams SET team_prompt = ?1 WHERE id = 'backend_team' AND team_prompt = ''",
        ["Focused backend pair: 1) Tech Lead plans and reviews 2) Engineer implements with TDD. Communication: Sequential — plan first, then implement."]);

    let _ = conn.execute("UPDATE teams SET team_prompt = ?1 WHERE id = 'qa_team' AND team_prompt = ''",
        ["Quality-focused pair: 1) Engineer implements features 2) QA writes independent tests and validates. Communication: Parallel — QA can write test specs while Engineer implements."]);
    Ok(())
}

/// Initialize project DB (squad_sessions, tickets, workers, etc.)
pub fn init_project_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(PROJECT_SCHEMA)?;
    // Migrations
    let _ = conn.execute("ALTER TABLE squad_sessions ADD COLUMN is_default INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE squad_sessions ADD COLUMN base_branch TEXT", []);
    let _ = conn.execute("ALTER TABLE squad_sessions ADD COLUMN base_commit TEXT", []);
    let _ = conn.execute("ALTER TABLE squad_sessions ADD COLUMN last_active_at INTEGER", []);
    let _ = conn.execute("ALTER TABLE squad_sessions ADD COLUMN max_iterations INTEGER", []);
    let _ = conn.execute("ALTER TABLE tickets ADD COLUMN origin_session TEXT", []);
    let _ = conn.execute("ALTER TABLE tickets ADD COLUMN base_commit TEXT", []);
    let _ = conn.execute("ALTER TABLE tickets ADD COLUMN blocked_by TEXT DEFAULT '[]'", []);
    let _ = conn.execute("ALTER TABLE tickets ADD COLUMN merge_status TEXT DEFAULT 'pending'", []);
    let _ = conn.execute("ALTER TABLE tickets ADD COLUMN structure_plan TEXT", []);
    let _ = conn.execute("ALTER TABLE tickets ADD COLUMN is_checkpoint INTEGER DEFAULT 0", []);
    Ok(())
}

/// Legacy: initialize all tables in one DB (for backward compatibility during migration)
pub fn init_db(conn: &Connection) -> Result<()> {
    init_global_db(conn)?;
    init_project_db(conn)?;
    Ok(())
}
