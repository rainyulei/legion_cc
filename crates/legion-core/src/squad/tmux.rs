//! Tmux integration for Legion - session/pane management

use anyhow::{Context, Result};
use std::process::Command;
use tracing::{debug, info};

/// Layout info for a single-mode tmux session (sidebar + main)
pub struct SingleLayout {
    pub session_name: String,
    pub sidebar_pane: String,
    pub main_pane: String,
}

/// Layout info for a squad tmux session
pub struct TmuxLayout {
    pub session_name: String,
    pub leader_pane: String,
    pub worker_panes: Vec<String>,
}

/// Get the path to the current legion executable
pub fn legion_exe() -> Result<String> {
    std::env::current_exe()
        .context("Failed to get current executable path")
        .map(|p| p.to_string_lossy().to_string())
}

/// Create a single-mode layout: narrow sidebar (left) + main Claude pane (right)
pub fn create_single_layout(session_name: &str, sidebar_width: u16) -> Result<SingleLayout> {
    // Create session
    Command::new("tmux")
        .args(["new-session", "-d", "-s", session_name])
        .output()
        .context("Failed to create tmux session")?;

    // The initial pane will become the main (right) pane.
    // Split horizontally to create sidebar (left).
    Command::new("tmux")
        .args([
            "split-window",
            "-h",
            "-b", // insert left of current pane
            "-l",
            &sidebar_width.to_string(),
            "-t",
            &format!("{}:0.0", session_name),
        ])
        .output()
        .context("Failed to split for sidebar")?;

    let panes = get_pane_ids(session_name)?;
    // After split-window -h -b: pane 0 = sidebar (left), pane 1 = main (right)
    let sidebar_pane = panes.first().cloned().unwrap_or_default();
    let main_pane = panes.get(1).cloned().unwrap_or_default();

    // Focus the main pane (Claude Code)
    Command::new("tmux")
        .args(["select-pane", "-t", &main_pane])
        .output()
        .ok();

    info!(
        "Created single layout: sidebar={}, main={}",
        sidebar_pane, main_pane
    );

    Ok(SingleLayout {
        session_name: session_name.to_string(),
        sidebar_pane,
        main_pane,
    })
}

/// Create a squad layout: left leader_width_pct% leader + right vertically stacked workers
pub fn create_squad_layout(
    session_name: &str,
    worker_count: u32,
    leader_width_pct: u32,
) -> Result<TmuxLayout> {
    Command::new("tmux")
        .args(["new-session", "-d", "-s", session_name])
        .output()
        .context("Failed to create tmux session")?;

    // Split horizontally: leader (left) | first worker (right)
    let worker_pct = 100 - leader_width_pct;
    Command::new("tmux")
        .args([
            "split-window",
            "-h",
            "-p",
            &worker_pct.to_string(),
            "-t",
            &format!("{}:0.0", session_name),
        ])
        .output()
        .context("Failed to split window horizontally")?;

    // Split the right pane vertically for additional workers
    for _ in 1..worker_count {
        let panes = get_pane_ids(session_name)?;
        if let Some(last_pane) = panes.last() {
            Command::new("tmux")
                .args(["split-window", "-v", "-t", last_pane])
                .output()
                .context("Failed to split worker pane")?;
        }
    }

    // Even out worker pane heights
    for pane in get_pane_ids(session_name)?.iter().skip(1) {
        Command::new("tmux")
            .args(["select-layout", "-t", pane, "even-vertical"])
            .output()
            .ok();
    }

    // Re-apply leader width
    let panes = get_pane_ids(session_name)?;
    if let Some(leader) = panes.first() {
        let width_output = Command::new("tmux")
            .args([
                "display-message",
                "-t",
                session_name,
                "-p",
                "#{window_width}",
            ])
            .output()?;
        let window_width: u32 = String::from_utf8_lossy(&width_output.stdout)
            .trim()
            .parse()
            .unwrap_or(200);
        let leader_cols = window_width * leader_width_pct / 100;
        Command::new("tmux")
            .args(["resize-pane", "-t", leader, "-x", &leader_cols.to_string()])
            .output()
            .ok();
    }

    // Select the leader pane as active
    if let Some(leader) = panes.first() {
        Command::new("tmux")
            .args(["select-pane", "-t", leader])
            .output()
            .ok();
    }

    let panes = get_pane_ids(session_name)?;

    let layout = TmuxLayout {
        session_name: session_name.to_string(),
        leader_pane: panes.first().cloned().unwrap_or_default(),
        worker_panes: panes.into_iter().skip(1).collect(),
    };

    info!(
        "Created squad layout: leader={}, workers={:?}",
        layout.leader_pane, layout.worker_panes
    );

    Ok(layout)
}

/// Configure minimal styling for single mode (no pane borders, clean look)
pub fn configure_single_style(session_name: &str) -> Result<()> {
    // Hide pane borders status (sidebar handles the info display)
    tmux_set_option(session_name, "pane-border-status", "off")?;
    // Subtle pane border line
    tmux_set_option(session_name, "pane-border-style", "fg=colour238")?;
    tmux_set_option(session_name, "pane-active-border-style", "fg=colour238")?;
    // Hide tmux status bar entirely (sidebar is our status display)
    tmux_set_option(session_name, "status", "off")?;
    // Enable mouse
    tmux_set_option(session_name, "mouse", "on")?;

    debug!("Configured single style for session {}", session_name);
    Ok(())
}

/// Set a pane's border title
pub fn set_pane_title(pane_id: &str, title: &str) -> Result<()> {
    Command::new("tmux")
        .args(["select-pane", "-t", pane_id, "-T", title])
        .output()
        .context("Failed to set pane title")?;
    debug!("Set pane {} title: {}", pane_id, title);
    Ok(())
}

/// Configure pane borders to show titles with nice styling (for squad mode)
pub fn configure_pane_borders(session_name: &str) -> Result<()> {
    tmux_set_option(session_name, "pane-border-status", "top")?;
    tmux_set_option(
        session_name,
        "pane-border-format",
        "#{?pane_active,#[fg=cyan#,bold] \u{25cf} #{pane_title} ,#[fg=colour240] \u{25cb} #{pane_title} }",
    )?;
    tmux_set_option(session_name, "pane-active-border-style", "fg=cyan")?;
    tmux_set_option(session_name, "pane-border-style", "fg=colour238")?;
    tmux_set_option(session_name, "mouse", "on")?;

    debug!("Configured pane borders for session {}", session_name);
    Ok(())
}

/// Configure tmux status bars (for squad mode)
pub fn configure_status_bars(
    session_name: &str,
    provider_name: &str,
    model_name: &str,
    mode: &str,
) -> Result<()> {
    tmux_set_option(session_name, "status-position", "top")?;
    tmux_set_option(session_name, "status", "2")?;

    let status_left = format!(
        "#[fg=black,bg=cyan,bold] Legion #[fg=cyan,bg=colour236]\u{e0b0}#[fg=white,bg=colour236] {} #[fg=colour240]\u{2192} #[fg=magenta]{} #[fg=colour236,bg=default]\u{e0b0}",
        provider_name, model_name
    );
    tmux_set_option(
        session_name,
        "status-format[0]",
        &format!(
            "#[align=left]{}#[align=right]#[fg=colour240] {} ",
            status_left, mode
        ),
    )?;

    tmux_set_option(session_name, "status-format[1]",
        "#[align=left]#[fg=colour240] #[fg=yellow]prefix+p#[fg=colour240]: Switch Provider/Model  #[fg=yellow]prefix+d#[fg=colour240]: Detach  #[fg=yellow]Ctrl+C#[fg=colour240]: Quit"
    )?;

    tmux_set_option(session_name, "status-style", "bg=default")?;
    tmux_set_option(session_name, "window-status-format", "")?;
    tmux_set_option(session_name, "window-status-current-format", "")?;
    tmux_set_option(session_name, "status-left", "")?;
    tmux_set_option(session_name, "status-right", "")?;
    tmux_set_option(session_name, "status-left-length", "0")?;
    tmux_set_option(session_name, "status-right-length", "0")?;

    debug!("Configured status bars for session {}", session_name);
    Ok(())
}

/// Bind prefix+p to launch the legion popup for provider/model switching
pub fn bind_popup_key(_session_name: &str, control_port: u16) -> Result<()> {
    let exe = legion_exe()?;
    Command::new("tmux")
        .args([
            "bind-key",
            "-T",
            "prefix",
            "p",
            "display-popup",
            "-E",
            "-w",
            "50%",
            "-h",
            "50%",
            &format!("{} popup --control-port {}", exe, control_port),
        ])
        .output()
        .context("Failed to bind popup key")?;

    info!(
        "Bound prefix+p to legion popup (control port {})",
        control_port
    );
    Ok(())
}

/// Send the sidebar command to a pane
pub fn send_sidebar_command(
    session_name: &str,
    pane_id: &str,
    proxy_port: u16,
    control_port: u16,
    mode: &str,
) -> Result<()> {
    let exe = legion_exe()?;
    let cmd = format!(
        "{} sidebar --control-port {} --proxy-port {} --mode {}",
        exe, control_port, proxy_port, mode
    );
    send_command_to_pane(session_name, pane_id, &cmd)
}

/// Send the legion start command to a pane (uses full binary path)
pub fn send_legion_start(
    session_name: &str,
    pane_id: &str,
    proxy_port: u16,
    control_port: u16,
) -> Result<()> {
    let exe = legion_exe()?;
    let cmd = format!(
        "{} start --port {} --control-port {} --no-tmux",
        exe, proxy_port, control_port
    );
    send_command_to_pane(session_name, pane_id, &cmd)
}

/// Send a command to start claude in a pane with the proper environment
pub fn send_claude_command(
    session_name: &str,
    pane_id: &str,
    proxy_port: u16,
    control_port: u16,
) -> Result<()> {
    let cmd = format!(
        "ANTHROPIC_BASE_URL=http://127.0.0.1:{} LEGION_CONTROL_PORT={} claude",
        proxy_port, control_port
    );
    send_command_to_pane(session_name, pane_id, &cmd)
}

/// Send a shell command to a specific pane
pub fn send_command_to_pane(session: &str, pane: &str, command: &str) -> Result<()> {
    Command::new("tmux")
        .args(["send-keys", "-t", pane, command, "Enter"])
        .output()
        .context(format!(
            "Failed to send command to pane {} in session {}",
            pane, session
        ))?;
    debug!("Sent command to {}: {}", pane, command);
    Ok(())
}

/// Attach to a tmux session
pub fn attach_session(session_name: &str) -> Result<()> {
    Command::new("tmux")
        .args(["attach-session", "-t", session_name])
        .status()
        .context("Failed to attach to tmux session")?;
    Ok(())
}

/// Kill a tmux session
pub fn kill_session(session_name: &str) -> Result<()> {
    Command::new("tmux")
        .args(["kill-session", "-t", session_name])
        .output()
        .context("Failed to kill tmux session")?;
    Ok(())
}

/// Get pane IDs for a session
fn get_pane_ids(session_name: &str) -> Result<Vec<String>> {
    let output = Command::new("tmux")
        .args(["list-panes", "-t", session_name, "-F", "#{pane_id}"])
        .output()
        .context("Failed to list panes")?;

    let panes: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(String::from)
        .collect();

    Ok(panes)
}

/// Set a tmux session option
fn tmux_set_option(session_name: &str, option: &str, value: &str) -> Result<()> {
    Command::new("tmux")
        .args(["set-option", "-t", session_name, option, value])
        .output()
        .context(format!("Failed to set tmux option {} = {}", option, value))?;
    Ok(())
}
