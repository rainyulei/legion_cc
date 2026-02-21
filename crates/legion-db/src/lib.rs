pub mod schema;
pub mod repo;

use anyhow::Result;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

pub use repo::{FileDiffSummary, PaneConfig, Provider, Repository, Role, Session, SquadSession, Team, TicketDiffRow, TicketRow};

/// Global DB path: ~/Library/Application Support/legion/legion.db
pub fn get_db_path() -> PathBuf {
    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("legion");
    std::fs::create_dir_all(&data_dir).ok();
    data_dir.join("legion.db")
}

/// Open the global DB (providers, roles, teams, pane_configs)
pub fn open_db() -> Result<Repository> {
    let path = get_db_path();
    let conn = Connection::open(&path)?;
    schema::init_global_db(&conn)?;
    Ok(Repository::new(conn))
}

/// Open a project-local DB at <project_path>/.legion/legion.db
/// (squad_sessions, tickets, ticket_logs, ticket_diffs, workers, etc.)
pub fn open_project_db(project_path: &Path) -> Result<Repository> {
    let db_dir = project_path.join(".legion");
    std::fs::create_dir_all(&db_dir)?;
    let path = db_dir.join("legion.db");
    let conn = Connection::open(&path)?;
    schema::init_project_db(&conn)?;
    Ok(Repository::new(conn))
}
