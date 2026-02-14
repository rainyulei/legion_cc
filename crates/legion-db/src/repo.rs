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
            "INSERT OR REPLACE INTO squad_sessions (name, project_path, worker_count, status, created_at, completed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session.name,
                session.project_path,
                session.worker_count,
                session.status,
                session.created_at,
                session.completed_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_squad_session(&self, name: &str) -> Result<Option<SquadSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, project_path, worker_count, status, created_at, completed_at FROM squad_sessions WHERE name = ?"
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
            }))
        } else {
            Ok(None)
        }
    }

    pub fn list_squad_sessions(&self) -> Result<Vec<SquadSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, project_path, worker_count, status, created_at, completed_at FROM squad_sessions ORDER BY created_at DESC"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SquadSession {
                name: row.get(0)?,
                project_path: row.get(1)?,
                worker_count: row.get(2)?,
                status: row.get(3)?,
                created_at: row.get(4)?,
                completed_at: row.get(5)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn list_active_squad_sessions(&self) -> Result<Vec<SquadSession>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, project_path, worker_count, status, created_at, completed_at FROM squad_sessions WHERE status = 'active' ORDER BY created_at DESC"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(SquadSession {
                name: row.get(0)?,
                project_path: row.get(1)?,
                worker_count: row.get(2)?,
                status: row.get(3)?,
                created_at: row.get(4)?,
                completed_at: row.get(5)?,
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
        }).unwrap();

        // Insert a completed session
        repo.upsert_squad_session(&SquadSession {
            name: "done-squad".into(),
            project_path: "/tmp/b".into(),
            worker_count: 4,
            status: "completed".into(),
            created_at: 900,
            completed_at: Some(1500),
        }).unwrap();

        // list_squad_sessions returns both
        let all = repo.list_squad_sessions().unwrap();
        assert_eq!(all.len(), 2);

        // list_active_squad_sessions returns only active
        let active = repo.list_active_squad_sessions().unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].name, "active-squad");
    }
}
