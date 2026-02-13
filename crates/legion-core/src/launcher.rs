//! Claude Code launcher - spawns Claude as a native subprocess

use std::process::{Command, Stdio};

use anyhow::Result;
use tracing::info;

/// Launcher for Claude Code processes
pub struct ClaudeLauncher {
    /// Port of the proxy server
    pub proxy_port: u16,
    /// Port of the control API
    pub control_port: u16,
}

impl ClaudeLauncher {
    /// Create a new launcher
    pub fn new(proxy_port: u16, control_port: u16) -> Self {
        Self {
            proxy_port,
            control_port,
        }
    }

    /// Spawn Claude Code as a child process inheriting stdio (for direct terminal use)
    pub fn spawn_claude(&self) -> Result<std::process::Child> {
        info!(
            "Spawning Claude Code with proxy port {} and control port {}",
            self.proxy_port, self.control_port
        );

        let child = Command::new("claude")
            .envs(self.claude_env())
            .stdin(Stdio::inherit())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .spawn()?;

        Ok(child)
    }

    /// Get environment variables for Claude Code
    pub fn claude_env(&self) -> Vec<(String, String)> {
        vec![
            (
                "ANTHROPIC_BASE_URL".to_string(),
                format!("http://127.0.0.1:{}", self.proxy_port),
            ),
            (
                "LEGION_CONTROL_PORT".to_string(),
                self.control_port.to_string(),
            ),
        ]
    }

    /// Get the command string for launching claude in a tmux pane
    pub fn claude_command(&self) -> String {
        format!(
            "ANTHROPIC_BASE_URL=http://127.0.0.1:{} LEGION_CONTROL_PORT={} claude",
            self.proxy_port, self.control_port
        )
    }
}

/// Check if tmux is available on the system
pub fn tmux_available() -> bool {
    Command::new("tmux")
        .arg("-V")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Check if we are currently inside a tmux session
pub fn in_tmux() -> bool {
    std::env::var("TMUX").is_ok()
}
