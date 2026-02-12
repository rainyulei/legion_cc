use anyhow::Result;
use std::process::Command;

pub struct TmuxLayout {
    pub session_name: String,
    pub leader_pane: String,
    pub worker_panes: Vec<String>,
}

pub fn create_squad_layout(session_name: &str, worker_count: u32) -> Result<TmuxLayout> {
    // Create new tmux session
    Command::new("tmux")
        .args(["new-session", "-d", "-s", session_name])
        .output()?;

    // Split vertically for leader (left) and workers (right)
    Command::new("tmux")
        .args(["split-window", "-h", "-t", session_name])
        .output()?;

    // Split workers horizontally
    for _i in 1..worker_count {
        Command::new("tmux")
            .args([
                "split-window",
                "-v",
                "-t",
                &format!("{}:0.1", session_name),
            ])
            .output()?;
    }

    // Collect pane IDs
    let output = Command::new("tmux")
        .args(["list-panes", "-t", session_name, "-F", "#{pane_id}"])
        .output()?;

    let panes: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(String::from)
        .collect();

    Ok(TmuxLayout {
        session_name: session_name.to_string(),
        leader_pane: panes.first().cloned().unwrap_or_default(),
        worker_panes: panes.into_iter().skip(1).collect(),
    })
}

pub fn send_command_to_pane(session: &str, pane: &str, command: &str) -> Result<()> {
    Command::new("tmux")
        .args([
            "send-keys",
            "-t",
            &format!("{}:{}", session, pane),
            command,
            "Enter",
        ])
        .output()?;
    Ok(())
}

pub fn attach_session(session_name: &str) -> Result<()> {
    Command::new("tmux")
        .args(["attach-session", "-t", session_name])
        .status()?;
    Ok(())
}

pub fn kill_session(session_name: &str) -> Result<()> {
    Command::new("tmux")
        .args(["kill-session", "-t", session_name])
        .output()?;
    Ok(())
}
