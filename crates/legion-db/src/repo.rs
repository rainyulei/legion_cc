use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub api_format: String,
    pub models: Option<Vec<String>>,
    pub is_default: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneConfig {
    pub pane_label: String,
    pub provider_id: String,
    pub model: Option<String>,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SquadSession {
    pub name: String,
    pub project_path: String,
    pub worker_count: i64,
    pub status: String,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: Option<String>,
    pub project_path: Option<String>,
    pub claude_session_file: Option<String>,
    pub provider_id: Option<String>,
    pub created_at: i64,
    pub last_active_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TicketRow {
    pub id: i64,
    pub session_name: String,
    pub title: String,
    pub prompt: String,
    pub context: Option<String>,
    pub criteria: Option<String>,
    pub status: String,
    pub assigned_worker: Option<i64>,
    pub team_mode: String,
    pub iteration: i64,
    pub max_iterations: i64,
    pub feedback: Option<String>,
    pub summary: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub origin_session: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiffSummary {
    pub path: String,
    pub status: String,
    pub additions: usize,
    pub deletions: usize,
}

pub struct TicketDiffRow {
    pub ticket_id: i64,
    pub session_name: String,
    pub diff_content: String,
    pub file_summary: Vec<FileDiffSummary>,
    pub cached_at: i64,
}

pub struct Repository {
    conn: Connection,
}

impl Repository {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }

    // Provider methods
    pub fn list_providers(&self) -> Result<Vec<Provider>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, base_url, api_key, api_format, models, is_default, created_at FROM providers ORDER BY name"
        )?;
        let rows = stmt.query_map([], |row| {
            let models_json: Option<String> = row.get(5)?;
            let models = models_json.and_then(|s| serde_json::from_str(&s).ok());
            Ok(Provider {
                id: row.get(0)?,
                name: row.get(1)?,
                base_url: row.get(2)?,
                api_key: row.get(3)?,
                api_format: row.get(4)?,
                models,
                is_default: row.get::<_, i32>(6)? != 0,
                created_at: row.get(7)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn get_provider(&self, id: &str) -> Result<Option<Provider>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, base_url, api_key, api_format, models, is_default, created_at FROM providers WHERE id = ?"
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            let models_json: Option<String> = row.get(5)?;
            let models = models_json.and_then(|s| serde_json::from_str(&s).ok());
            Ok(Some(Provider {
                id: row.get(0)?,
                name: row.get(1)?,
                base_url: row.get(2)?,
                api_key: row.get(3)?,
                api_format: row.get(4)?,
                models,
                is_default: row.get::<_, i32>(6)? != 0,
                created_at: row.get(7)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn insert_provider(&self, provider: &Provider) -> Result<()> {
        let models_json = provider.models.as_ref().map(|m| serde_json::to_string(m).unwrap());
        self.conn.execute(
            "INSERT INTO providers (id, name, base_url, api_key, api_format, models, is_default, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                provider.id,
                provider.name,
                provider.base_url,
                provider.api_key,
                provider.api_format,
                models_json,
                provider.is_default as i32,
                provider.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn upsert_provider(&self, provider: &Provider) -> Result<()> {
        let models_json = provider.models.as_ref().map(|m| serde_json::to_string(m).unwrap());
        self.conn.execute(
            "INSERT OR REPLACE INTO providers (id, name, base_url, api_key, api_format, models, is_default, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                provider.id,
                provider.name,
                provider.base_url,
                provider.api_key,
                provider.api_format,
                models_json,
                provider.is_default as i32,
                provider.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_provider_models(&self, id: &str, models: &[String]) -> Result<()> {
        let models_json = serde_json::to_string(models)?;
        self.conn.execute(
            "UPDATE providers SET models = ?1 WHERE id = ?2",
            params![models_json, id],
        )?;
        Ok(())
    }

    pub fn get_default_provider(&self) -> Result<Option<Provider>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, base_url, api_key, api_format, models, is_default, created_at FROM providers WHERE is_default = 1 LIMIT 1"
        )?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let models_json: Option<String> = row.get(5)?;
            let models = models_json.and_then(|s| serde_json::from_str(&s).ok());
            Ok(Some(Provider {
                id: row.get(0)?,
                name: row.get(1)?,
                base_url: row.get(2)?,
                api_key: row.get(3)?,
                api_format: row.get(4)?,
                models,
                is_default: true,
                created_at: row.get(7)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn set_default_provider(&self, id: &str) -> Result<()> {
        self.conn.execute("UPDATE providers SET is_default = 0", [])?;
        self.conn.execute("UPDATE providers SET is_default = 1 WHERE id = ?", params![id])?;
        Ok(())
    }

    // Session methods
    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, project_path, claude_session_file, provider_id, created_at, last_active_at FROM sessions ORDER BY last_active_at DESC"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Session {
                id: row.get(0)?,
                name: row.get(1)?,
                project_path: row.get(2)?,
                claude_session_file: row.get(3)?,
                provider_id: row.get(4)?,
                created_at: row.get(5)?,
                last_active_at: row.get(6)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn insert_session(&self, session: &Session) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, name, project_path, claude_session_file, provider_id, created_at, last_active_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session.id,
                session.name,
                session.project_path,
                session.claude_session_file,
                session.provider_id,
                session.created_at,
                session.last_active_at,
            ],
        )?;
        Ok(())
    }

    pub fn update_session_active(&self, id: &str, timestamp: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET last_active_at = ?1 WHERE id = ?2",
            params![timestamp, id],
        )?;
        Ok(())
    }

    // Pane config methods

    pub fn upsert_pane_config(&self, config: &PaneConfig) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO pane_configs (pane_label, provider_id, model, updated_at) VALUES (?1, ?2, ?3, ?4)",
            params![config.pane_label, config.provider_id, config.model, config.updated_at],
        )?;
        Ok(())
    }

    pub fn get_pane_config(&self, pane_label: &str) -> Result<Option<PaneConfig>> {
        let mut stmt = self.conn.prepare(
            "SELECT pane_label, provider_id, model, updated_at FROM pane_configs WHERE pane_label = ?"
        )?;
        let mut rows = stmt.query(params![pane_label])?;
        if let Some(row) = rows.next()? {
            Ok(Some(PaneConfig {
                pane_label: row.get(0)?,
                provider_id: row.get(1)?,
                model: row.get(2)?,
                updated_at: row.get(3)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn list_pane_configs(&self) -> Result<Vec<PaneConfig>> {
        let mut stmt = self.conn.prepare(
            "SELECT pane_label, provider_id, model, updated_at FROM pane_configs ORDER BY pane_label"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PaneConfig {
                pane_label: row.get(0)?,
                provider_id: row.get(1)?,
                model: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    // Squad session methods

    pub fn upsert_squad_session(&self, session: &SquadSession) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO squad_sessions (name, project_path, worker_count, status, created_at, completed_at, is_default) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                session.name,
                session.project_path,
                session.worker_count,
                session.status,
                session.created_at,
                session.completed_at,
                session.is_default as i32,
            ],
        )?;
        Ok(())
    }

    pub fn get_squad_session(&self, name: &str) -> Result<Option<SquadSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, project_path, worker_count, status, created_at, completed_at, is_default FROM squad_sessions WHERE name = ?"
        )?;
        let mut rows = stmt.query(params![name])?;
        if let Some(row) = rows.next()? {
            Ok(Some(SquadSession {
                name: row.get(0)?,
                project_path: row.get(1)?,
                worker_count: row.get(2)?,
                status: row.get(3)?,
                created_at: row.get(4)?,
                completed_at: row.get(5)?,
                is_default: row.get::<_, i32>(6)? != 0,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn list_squad_sessions(&self) -> Result<Vec<SquadSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, project_path, worker_count, status, created_at, completed_at, is_default FROM squad_sessions ORDER BY created_at DESC"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SquadSession {
                name: row.get(0)?,
                project_path: row.get(1)?,
                worker_count: row.get(2)?,
                status: row.get(3)?,
                created_at: row.get(4)?,
                completed_at: row.get(5)?,
                is_default: row.get::<_, i32>(6)? != 0,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn list_active_squad_sessions(&self) -> Result<Vec<SquadSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, project_path, worker_count, status, created_at, completed_at, is_default FROM squad_sessions WHERE status = 'active' ORDER BY created_at DESC"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SquadSession {
                name: row.get(0)?,
                project_path: row.get(1)?,
                worker_count: row.get(2)?,
                status: row.get(3)?,
                created_at: row.get(4)?,
                completed_at: row.get(5)?,
                is_default: row.get::<_, i32>(6)? != 0,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn complete_squad_session(&self, name: &str, completed_at: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE squad_sessions SET status = 'completed', completed_at = ?1 WHERE name = ?2",
            params![completed_at, name],
        )?;
        Ok(())
    }

    pub fn delete_squad_session(&self, name: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM squad_sessions WHERE name = ?",
            params![name],
        )?;
        Ok(())
    }

    // Ticket methods

    pub fn insert_ticket(&self, ticket: &TicketRow) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO tickets (id, session_name, title, prompt, context, criteria, status, assigned_worker, team_mode, iteration, max_iterations, feedback, summary, created_at, updated_at, origin_session) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                ticket.id,
                ticket.session_name,
                ticket.title,
                ticket.prompt,
                ticket.context,
                ticket.criteria,
                ticket.status,
                ticket.assigned_worker,
                ticket.team_mode,
                ticket.iteration,
                ticket.max_iterations,
                ticket.feedback,
                ticket.summary,
                ticket.created_at,
                ticket.updated_at,
                ticket.origin_session,
            ],
        )?;
        Ok(())
    }

    pub fn update_ticket(&self, ticket: &TicketRow) -> Result<()> {
        self.conn.execute(
            "UPDATE tickets SET status = ?1, assigned_worker = ?2, iteration = ?3, feedback = ?4, summary = ?5, updated_at = ?6 WHERE id = ?7 AND session_name = ?8",
            params![
                ticket.status,
                ticket.assigned_worker,
                ticket.iteration,
                ticket.feedback,
                ticket.summary,
                ticket.updated_at,
                ticket.id,
                ticket.session_name,
            ],
        )?;
        Ok(())
    }

    pub fn list_tickets_by_session(&self, session_name: &str) -> Result<Vec<TicketRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_name, title, prompt, context, criteria, status, assigned_worker, team_mode, iteration, max_iterations, feedback, summary, created_at, updated_at, origin_session FROM tickets WHERE session_name = ? ORDER BY id"
        )?;
        let rows = stmt.query_map(params![session_name], |row| {
            Ok(TicketRow {
                id: row.get(0)?,
                session_name: row.get(1)?,
                title: row.get(2)?,
                prompt: row.get(3)?,
                context: row.get(4)?,
                criteria: row.get(5)?,
                status: row.get(6)?,
                assigned_worker: row.get(7)?,
                team_mode: row.get(8)?,
                iteration: row.get(9)?,
                max_iterations: row.get(10)?,
                feedback: row.get(11)?,
                summary: row.get(12)?,
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
                origin_session: row.get(15)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn max_ticket_id(&self, session_name: &str) -> Result<usize> {
        let id: Option<i64> = self.conn.query_row(
            "SELECT MAX(id) FROM tickets WHERE session_name = ?",
            params![session_name],
            |row| row.get(0),
        )?;
        Ok(id.unwrap_or(0) as usize)
    }

    // Ticket log methods

    pub fn append_ticket_log(&self, ticket_id: i64, session_name: &str, content: &str, created_at: i64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO ticket_logs (ticket_id, session_name, content, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![ticket_id, session_name, content, created_at],
        )?;
        Ok(())
    }

    pub fn get_ticket_logs(&self, ticket_id: i64, session_name: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT content FROM ticket_logs WHERE ticket_id = ?1 AND session_name = ?2 ORDER BY id"
        )?;
        let rows = stmt.query_map(params![ticket_id, session_name], |row| {
            row.get::<_, String>(0)
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Delete a ticket and its logs by id and session
    pub fn delete_ticket(&self, ticket_id: i64, session_name: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM ticket_logs WHERE ticket_id = ?1 AND session_name = ?2",
            params![ticket_id, session_name],
        )?;
        self.conn.execute(
            "DELETE FROM ticket_diffs WHERE ticket_id = ?1",
            params![ticket_id],
        )?;
        self.conn.execute(
            "DELETE FROM tickets WHERE id = ?1 AND session_name = ?2",
            params![ticket_id, session_name],
        )?;
        Ok(())
    }

    pub fn get_default_squad_session(&self, project_path: &str) -> Result<Option<SquadSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, project_path, worker_count, status, created_at, completed_at, is_default FROM squad_sessions WHERE is_default = 1 AND project_path = ? LIMIT 1"
        )?;
        let mut rows = stmt.query(params![project_path])?;
        if let Some(row) = rows.next()? {
            Ok(Some(SquadSession {
                name: row.get(0)?,
                project_path: row.get(1)?,
                worker_count: row.get(2)?,
                status: row.get(3)?,
                created_at: row.get(4)?,
                completed_at: row.get(5)?,
                is_default: row.get::<_, i32>(6)? != 0,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn delete_session_tickets(&self, session_name: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM ticket_logs WHERE session_name = ?",
            params![session_name],
        )?;
        self.conn.execute(
            "DELETE FROM ticket_diffs WHERE session_name = ?",
            params![session_name],
        )?;
        self.conn.execute(
            "DELETE FROM tickets WHERE session_name = ?",
            params![session_name],
        )?;
        Ok(())
    }

    pub fn migrate_tickets_to_session(&self, from_session: &str, to_session: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE tickets SET session_name = ?1, origin_session = ?2 WHERE session_name = ?2",
            params![to_session, from_session],
        )?;
        self.conn.execute(
            "UPDATE ticket_logs SET session_name = ?1 WHERE session_name = ?2",
            params![to_session, from_session],
        )?;
        Ok(())
    }

    pub fn count_pending_tickets(&self, session_name: &str) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tickets WHERE session_name = ? AND status IN ('queued', 'in_progress')",
            params![session_name],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn count_tickets(&self, session_name: &str) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tickets WHERE session_name = ?",
            params![session_name],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn count_ticket_logs(&self, session_name: &str) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM ticket_logs WHERE session_name = ?",
            params![session_name],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    // Ticket diff methods

    pub fn save_ticket_diff(
        &self,
        ticket_id: i64,
        session_name: &str,
        diff_content: &str,
        file_summary_json: &str,
        cached_at: i64,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO ticket_diffs (ticket_id, session_name, diff_content, file_summary, cached_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ticket_id, session_name, diff_content, file_summary_json, cached_at],
        )?;
        Ok(())
    }

    pub fn get_ticket_diff(&self, ticket_id: i64) -> Result<Option<TicketDiffRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT ticket_id, session_name, diff_content, file_summary, cached_at FROM ticket_diffs WHERE ticket_id = ?1"
        )?;
        let mut rows = stmt.query(params![ticket_id])?;
        if let Some(row) = rows.next()? {
            let summary_json: String = row.get(3)?;
            let file_summary: Vec<FileDiffSummary> = serde_json::from_str(&summary_json).unwrap_or_default();
            Ok(Some(TicketDiffRow {
                ticket_id: row.get(0)?,
                session_name: row.get(1)?,
                diff_content: row.get(2)?,
                file_summary,
                cached_at: row.get(4)?,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn delete_ticket_diff(&self, ticket_id: i64) -> Result<()> {
        self.conn.execute(
            "DELETE FROM ticket_diffs WHERE ticket_id = ?1",
            params![ticket_id],
        )?;
        Ok(())
    }

    pub fn delete_session_ticket_diffs(&self, session_name: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM ticket_diffs WHERE session_name = ?1",
            params![session_name],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::init_db;
    use rusqlite::Connection;

    fn test_repo() -> Repository {
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        Repository::new(conn)
    }

    #[test]
    fn pane_config_upsert_and_get() {
        let repo = test_repo();

        let config = PaneConfig {
            pane_label: "Leader".into(),
            provider_id: "provider-abc".into(),
            model: Some("claude-opus-4-6".into()),
            updated_at: 100,
        };
        repo.upsert_pane_config(&config).unwrap();

        let loaded = repo.get_pane_config("Leader").unwrap().unwrap();
        assert_eq!(loaded.provider_id, "provider-abc");
        assert_eq!(loaded.model.as_deref(), Some("claude-opus-4-6"));

        // Upsert overwrites
        let config2 = PaneConfig {
            pane_label: "Leader".into(),
            provider_id: "provider-xyz".into(),
            model: Some("gpt-5".into()),
            updated_at: 200,
        };
        repo.upsert_pane_config(&config2).unwrap();

        let loaded2 = repo.get_pane_config("Leader").unwrap().unwrap();
        assert_eq!(loaded2.provider_id, "provider-xyz");
        assert_eq!(loaded2.model.as_deref(), Some("gpt-5"));
    }

    #[test]
    fn pane_config_list_and_missing() {
        let repo = test_repo();

        // Empty initially
        let all = repo.list_pane_configs().unwrap();
        assert!(all.is_empty());

        // Missing key returns None
        assert!(repo.get_pane_config("Worker 99").unwrap().is_none());

        // Insert two
        repo.upsert_pane_config(&PaneConfig {
            pane_label: "Worker 1".into(),
            provider_id: "__native__".into(),
            model: None,
            updated_at: 100,
        }).unwrap();
        repo.upsert_pane_config(&PaneConfig {
            pane_label: "Leader".into(),
            provider_id: "p1".into(),
            model: Some("m1".into()),
            updated_at: 100,
        }).unwrap();

        let all = repo.list_pane_configs().unwrap();
        assert_eq!(all.len(), 2);
        // Sorted by label: Leader < Worker 1
        assert_eq!(all[0].pane_label, "Leader");
        assert_eq!(all[1].pane_label, "Worker 1");
        assert!(all[1].model.is_none());
    }

    #[test]
    fn squad_session_crud() {
        let repo = test_repo();

        // Insert
        let session = SquadSession {
            name: "my-squad".into(),
            project_path: "/tmp/project".into(),
            worker_count: 3,
            status: "active".into(),
            created_at: 1000,
            completed_at: None,
            is_default: false,
        };
        repo.upsert_squad_session(&session).unwrap();

        // Get
        let loaded = repo.get_squad_session("my-squad").unwrap().unwrap();
        assert_eq!(loaded.name, "my-squad");
        assert_eq!(loaded.project_path, "/tmp/project");
        assert_eq!(loaded.worker_count, 3);
        assert_eq!(loaded.status, "active");
        assert_eq!(loaded.created_at, 1000);
        assert!(loaded.completed_at.is_none());

        // List
        let all = repo.list_squad_sessions().unwrap();
        assert_eq!(all.len(), 1);

        // Complete
        repo.complete_squad_session("my-squad", 2000).unwrap();
        let loaded = repo.get_squad_session("my-squad").unwrap().unwrap();
        assert_eq!(loaded.status, "completed");
        assert_eq!(loaded.completed_at, Some(2000));

        // Delete
        repo.delete_squad_session("my-squad").unwrap();
        assert!(repo.get_squad_session("my-squad").unwrap().is_none());
        assert!(repo.list_squad_sessions().unwrap().is_empty());
    }

    #[test]
    fn squad_session_list_active_only() {
        let repo = test_repo();

        // Insert an active session
        repo.upsert_squad_session(&SquadSession {
            name: "active-squad".into(),
            project_path: "/tmp/a".into(),
            worker_count: 2,
            status: "active".into(),
            created_at: 1000,
            completed_at: None,
            is_default: false,
        }).unwrap();

        // Insert a completed session
        repo.upsert_squad_session(&SquadSession {
            name: "done-squad".into(),
            project_path: "/tmp/b".into(),
            worker_count: 4,
            status: "completed".into(),
            created_at: 900,
            completed_at: Some(1500),
            is_default: false,
        }).unwrap();

        // list_squad_sessions returns both
        let all = repo.list_squad_sessions().unwrap();
        assert_eq!(all.len(), 2);

        // list_active_squad_sessions returns only active
        let active = repo.list_active_squad_sessions().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].name, "active-squad");
    }

    #[test]
    fn default_squad_session() {
        let repo = test_repo();

        // No default session initially
        assert!(repo.get_default_squad_session("/tmp/project").unwrap().is_none());

        // Insert a non-default session
        repo.upsert_squad_session(&SquadSession {
            name: "regular".into(),
            project_path: "/tmp/project".into(),
            worker_count: 2,
            status: "active".into(),
            created_at: 1000,
            completed_at: None,
            is_default: false,
        }).unwrap();
        assert!(repo.get_default_squad_session("/tmp/project").unwrap().is_none());

        // Insert a default session
        repo.upsert_squad_session(&SquadSession {
            name: "default-squad".into(),
            project_path: "/tmp/project".into(),
            worker_count: 3,
            status: "active".into(),
            created_at: 2000,
            completed_at: None,
            is_default: true,
        }).unwrap();

        let default = repo.get_default_squad_session("/tmp/project").unwrap().unwrap();
        assert_eq!(default.name, "default-squad");
        assert!(default.is_default);

        // Different project_path returns None
        assert!(repo.get_default_squad_session("/tmp/other").unwrap().is_none());
    }

    #[test]
    fn delete_session_tickets_test() {
        let repo = test_repo();

        // Create a session and tickets
        repo.upsert_squad_session(&SquadSession {
            name: "sess1".into(),
            project_path: "/tmp/p".into(),
            worker_count: 1,
            status: "active".into(),
            created_at: 1000,
            completed_at: None,
            is_default: false,
        }).unwrap();

        let ticket = TicketRow {
            id: 1,
            session_name: "sess1".into(),
            title: "Task 1".into(),
            prompt: "Do something".into(),
            context: None,
            criteria: None,
            status: "queued".into(),
            assigned_worker: None,
            team_mode: "tech_lead_team".into(),
            iteration: 0,
            max_iterations: 5,
            feedback: None,
            summary: None,
            created_at: 1000,
            updated_at: 1000,
            origin_session: None,
        };
        repo.insert_ticket(&ticket).unwrap();
        repo.append_ticket_log(1, "sess1", "log entry", 1000).unwrap();

        assert_eq!(repo.count_tickets("sess1").unwrap(), 1);
        assert_eq!(repo.count_ticket_logs("sess1").unwrap(), 1);

        // Delete all tickets for the session
        repo.delete_session_tickets("sess1").unwrap();

        assert_eq!(repo.count_tickets("sess1").unwrap(), 0);
        assert_eq!(repo.count_ticket_logs("sess1").unwrap(), 0);
    }

    #[test]
    fn migrate_tickets_between_sessions() {
        let repo = test_repo();

        // Create two sessions
        for name in &["old-sess", "new-sess"] {
            repo.upsert_squad_session(&SquadSession {
                name: (*name).into(),
                project_path: "/tmp/p".into(),
                worker_count: 1,
                status: "active".into(),
                created_at: 1000,
                completed_at: None,
                is_default: false,
            }).unwrap();
        }

        // Insert tickets in old session
        for i in 1..=3 {
            repo.insert_ticket(&TicketRow {
                id: i,
                session_name: "old-sess".into(),
                title: format!("Task {}", i),
                prompt: "Do it".into(),
                context: None,
                criteria: None,
                status: "queued".into(),
                assigned_worker: None,
                team_mode: "tech_lead_team".into(),
                iteration: 0,
                max_iterations: 5,
                feedback: None,
                summary: None,
                created_at: 1000,
                updated_at: 1000,
                origin_session: None,
            }).unwrap();
            repo.append_ticket_log(i, "old-sess", &format!("log {}", i), 1000).unwrap();
        }

        assert_eq!(repo.count_tickets("old-sess").unwrap(), 3);
        assert_eq!(repo.count_tickets("new-sess").unwrap(), 0);

        // Migrate
        repo.migrate_tickets_to_session("old-sess", "new-sess").unwrap();

        assert_eq!(repo.count_tickets("old-sess").unwrap(), 0);
        assert_eq!(repo.count_tickets("new-sess").unwrap(), 3);
        assert_eq!(repo.count_ticket_logs("old-sess").unwrap(), 0);
        assert_eq!(repo.count_ticket_logs("new-sess").unwrap(), 3);

        // Verify origin_session is set
        let tickets = repo.list_tickets_by_session("new-sess").unwrap();
        for t in &tickets {
            assert_eq!(t.origin_session.as_deref(), Some("old-sess"));
        }

        // Verify pending count
        assert_eq!(repo.count_pending_tickets("new-sess").unwrap(), 3);
    }
}
