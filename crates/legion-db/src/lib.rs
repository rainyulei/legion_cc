pub mod schema;
pub mod repo;

use anyhow::Result;
use rusqlite::Connection;
use std::path::PathBuf;

pub use repo::{FileDiffSummary, PaneConfig, Provider, Repository, Role, Session, SquadSession, Team, TicketDiffRow, TicketRow};

pub fn get_db_path() -> PathBuf {
    let data_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("legion");
    std::fs::create_dir_all(&data_dir).ok();
    data_dir.join("legion.db")
}

pub fn open_db() -> Result<Repository> {
    let path = get_db_path();
    let conn = Connection::open(&path)?;
    schema::init_db(&conn)?;
    Ok(Repository::new(conn))
}
