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
}
