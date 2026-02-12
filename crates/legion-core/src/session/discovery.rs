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

    // Walk through projects directory
    for entry in std::fs::read_dir(&projects_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            // Look for .jsonl files
            for file_entry in std::fs::read_dir(&path)? {
                let file_entry = file_entry?;
                let file_path = file_entry.path();

                if file_path.extension().map(|e| e == "jsonl").unwrap_or(false) {
                    let metadata = std::fs::metadata(&file_path)?;
                    let modified: DateTime<Utc> = metadata.modified()?.into();

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
    let time_str = if elapsed.num_hours() < 1 {
        format!("{}m ago", elapsed.num_minutes())
    } else if elapsed.num_days() < 1 {
        format!("{}h ago", elapsed.num_hours())
    } else {
        format!("{}d ago", elapsed.num_days())
    };

    format!("{} ({})", session.project_path, time_str)
}
