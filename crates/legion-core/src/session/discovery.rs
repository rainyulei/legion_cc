use anyhow::Result;
use std::path::PathBuf;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct ClaudeSession {
    pub id: String,
    pub project_path: String,
    pub session_file: PathBuf,
    pub last_modified: DateTime<Utc>,
}

pub fn get_claude_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
}

pub fn discover_sessions() -> Result<Vec<ClaudeSession>> {
    let claude_dir = get_claude_dir();
    let projects_dir = claude_dir.join("projects");

    let mut sessions = Vec::new();

    if !projects_dir.exists() {
        return Ok(sessions);
    }

    // Walk through projects directory, skipping entries that fail to read
    for entry in std::fs::read_dir(&projects_dir)?.flatten() {
        let path = entry.path();

        if path.is_dir() {
            let dir_entries = match std::fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(_) => continue,
            };
            for file_entry in dir_entries.flatten() {
                let file_path = file_entry.path();

                if file_path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                    let Ok(metadata) = std::fs::metadata(&file_path) else { continue };
                    let Ok(modified_time) = metadata.modified() else { continue };
                    let modified: DateTime<Utc> = modified_time.into();

                    let id = file_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string();

                    let project_path = path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("unknown")
                        .to_string();

                    sessions.push(ClaudeSession {
                        id,
                        project_path,
                        session_file: file_path,
                        last_modified: modified,
                    });
                }
            }
        }
    }

    // Sort by last modified (most recent first)
    sessions.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));

    Ok(sessions)
}

pub fn get_session_display_name(session: &ClaudeSession) -> String {
    let elapsed = Utc::now() - session.last_modified;
    let time_str = if elapsed.num_minutes() < 1 {
        "just now".to_string()
    } else if elapsed.num_hours() < 1 {
        format!("{}m ago", elapsed.num_minutes())
    } else if elapsed.num_days() < 1 {
        format!("{}h ago", elapsed.num_hours())
    } else {
        format!("{}d ago", elapsed.num_days())
    };

    format!("{} ({})", session.project_path, time_str)
}
