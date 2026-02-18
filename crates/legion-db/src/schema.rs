use rusqlite::Connection;
use anyhow::Result;

pub const SCHEMA: &str = r#"
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

CREATE TABLE IF NOT EXISTS pane_configs (
    pane_label TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL,
    model TEXT,
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

pub fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
    // Migrations — safe to re-run (ignore "duplicate column" errors)
    let _ = conn.execute("ALTER TABLE squad_sessions ADD COLUMN is_default INTEGER NOT NULL DEFAULT 0", []);
    let _ = conn.execute("ALTER TABLE squad_sessions ADD COLUMN base_branch TEXT", []);
    let _ = conn.execute("ALTER TABLE squad_sessions ADD COLUMN base_commit TEXT", []);
    let _ = conn.execute("ALTER TABLE squad_sessions ADD COLUMN last_active_at INTEGER", []);
    let _ = conn.execute("ALTER TABLE squad_sessions ADD COLUMN max_iterations INTEGER", []);
    let _ = conn.execute("ALTER TABLE tickets ADD COLUMN origin_session TEXT", []);
    let _ = conn.execute("ALTER TABLE tickets ADD COLUMN base_commit TEXT", []);
    let _ = conn.execute("ALTER TABLE tickets ADD COLUMN blocked_by TEXT DEFAULT '[]'", []);
    let _ = conn.execute("ALTER TABLE tickets ADD COLUMN merge_status TEXT DEFAULT 'pending'", []);

    // Seed roles
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let _ = conn.execute("INSERT OR IGNORE INTO roles (id, name, description, prompt_template, is_builtin, created_at) VALUES (?1, ?2, ?3, ?4, 1, ?5)",
        ["tech_lead", "Tech Lead", "Technical lead responsible for architecture and planning",
         "You are the Tech Lead. Your approach: 1) Carefully analyze the requirements. 2) Design the architecture and identify components. 3) Break the work into concrete subtasks. 4) Review all code for correctness and edge cases. Focus on planning before implementation.", &now.to_string()]);

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
    Ok(())
}
