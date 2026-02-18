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
    Ok(())
}
