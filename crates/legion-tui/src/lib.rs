//! Legion TUI - Embedded terminal interface with provider/model switching

pub mod app;
pub mod claudemd;
pub mod input;
pub mod pty;
pub mod ui;
pub mod worktree;

use std::io;

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use app::App;
use input::{handle_key, handle_mouse, InputResult};
use ui::draw;

/// Run the full TUI with a single embedded Claude Code
pub async fn run(proxy_port: u16, control_port: u16) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and load providers from DB
    let mut app = App::new();
    app.load_from_db();

    // Calculate PTY size from terminal (minus header=1, footer=1, border=2)
    let size = terminal.size()?;
    app.term_size = (size.width, size.height);
    let pty_rows = size.height.saturating_sub(4);
    let pty_cols = size.width.saturating_sub(2);

    // Start Claude in single pane (no skip permissions - normal interactive flow)
    app.add_pane(pty_rows, pty_cols, proxy_port, control_port, "Claude Code".into(), false, None, None, None, None, false);

    // Main event loop
    let result = run_event_loop(&mut terminal, &mut app).await;

    // Kill child processes and restore terminal
    app.kill_all();
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    std::process::exit(if result.is_ok() { 0 } else { 1 });
}

/// Run the squad TUI with multiple embedded Claude Code panes
pub async fn run_squad(worker_count: u16, base_port: u16) -> Result<()> {
    // Detect project path
    let project_path = detect_project_path()
        .ok_or_else(|| anyhow::anyhow!("Not in a git repository — squad mode requires git"))?;

    // Session selection (before TUI starts)
    let (session_name, actual_workers, is_resume) =
        select_session_interactive(&project_path, worker_count)?;

    // Setup terminal with mouse support for divider dragging
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and load providers from DB
    let mut app = App::new();
    app.load_from_db();
    app.project_path = Some(project_path.clone());

    // Create or verify worktrees
    let worktree_paths = if is_resume {
        let mut paths = Vec::new();
        for i in 0..=actual_workers {
            let label = if i == 0 { "Leader".to_string() } else { format!("Worker {}", i) };
            let wt = worktree::pane_worktree_path(&project_path, &session_name, &label);
            if !worktree::worktree_exists(&wt) {
                tracing::warn!("Worktree missing for {}, recreating", label);
                let _ = worktree::create_worktree(&project_path, &session_name, &label);
            }
            paths.push(wt);
        }
        if let Ok(repo) = legion_db::open_db() {
            app.current_session = repo.get_squad_session(&session_name).ok().flatten();
        }
        paths
    } else {
        app.create_session(&session_name, actual_workers)?
    };

    // Cache terminal size
    let size = terminal.size()?;
    app.term_size = (size.width, size.height);

    // Calculate PTY sizes based on layout
    let content_height = size.height.saturating_sub(2);
    let leader_width = (size.width as u32 * app.leader_ratio as u32 / 100) as u16;
    let leader_pty_rows = content_height.saturating_sub(2);
    let leader_pty_cols = leader_width.saturating_sub(2);
    let worker_width = size.width.saturating_sub(leader_width).saturating_sub(1);
    let worker_height = content_height / actual_workers;
    let worker_pty_rows = worker_height.saturating_sub(2);
    let worker_pty_cols = worker_width.saturating_sub(2);

    // Generate system prompts
    let leader_prompt = claudemd::leader_instructions(actual_workers);
    let worker_prompts: Vec<String> = (1..=actual_workers)
        .map(|id| claudemd::worker_instructions(id))
        .collect();

    // Port assignments
    let orchestrate_port = base_port + 2000;
    let leader_proxy = base_port;
    let leader_control = base_port + 1000;

    // Spawn panes with worktree paths
    app.add_pane(
        leader_pty_rows, leader_pty_cols, leader_proxy, leader_control,
        "Leader".into(), false, None, Some(orchestrate_port), Some(&leader_prompt),
        Some(worktree_paths[0].as_path()), is_resume,
    );

    for i in 0..actual_workers {
        let proxy = base_port + i + 1;
        let control = base_port + 1000 + i + 1;
        let label = format!("Worker {}", i + 1);
        app.add_pane(
            worker_pty_rows, worker_pty_cols, proxy, control,
            label, true, Some(i + 1), Some(orchestrate_port),
            Some(&worker_prompts[i as usize]),
            Some(worktree_paths[1 + i as usize].as_path()), is_resume,
        );
    }

    // Start orchestration engine + API
    let engine = legion_core::OrchestrateEngine::new(actual_workers);
    app.orchestrate = Some(engine.clone());

    let orch_api = legion_core::OrchestrateApi::new(engine, orchestrate_port);
    let (orch_tx, orch_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        if let Err(e) = orch_api.start_with_signal(Some(orch_tx)).await {
            tracing::error!("Orchestrate API error on port {}: {}", orchestrate_port, e);
        }
    });
    orch_rx.await.ok();

    // Main event loop
    let result = run_event_loop(&mut terminal, &mut app).await;

    // Kill child processes and restore terminal
    app.kill_all();
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture);
    let _ = terminal.show_cursor();

    std::process::exit(if result.is_ok() { 0 } else { 1 });
}

/// Run the popup TUI only (for backward compat with `legion switch`)
pub async fn run_popup(_control_port: u16) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    app.load_from_db();
    app.toggle_popup(); // Start in popup mode

    let result = run_event_loop(&mut terminal, &mut app).await;

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    result
}

/// Detect the current project root via `git rev-parse --show-toplevel`
fn detect_project_path() -> Option<std::path::PathBuf> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Some(std::path::PathBuf::from(path))
    } else {
        None
    }
}

/// Pre-TUI session selection (runs before alternate screen)
fn select_session_interactive(_project_path: &std::path::Path, default_workers: u16) -> Result<(String, u16, bool)> {
    let sessions: Vec<legion_db::SquadSession> = legion_db::open_db()
        .and_then(|repo| repo.list_active_squad_sessions())
        .unwrap_or_default();

    if sessions.is_empty() {
        eprint!("Session name: ");
        let mut name = String::new();
        std::io::stdin().read_line(&mut name)?;
        let name = name.trim().to_string();
        if name.is_empty() {
            anyhow::bail!("Session name cannot be empty");
        }
        return Ok((name, default_workers, false));
    }

    eprintln!("\n  Active sessions:");
    for (i, s) in sessions.iter().enumerate() {
        let panes = 1 + s.worker_count;
        eprintln!("  {}. {} ({} panes)", i + 1, s.name, panes);
    }
    eprintln!("  {}. [New Session]", sessions.len() + 1);
    eprint!("\n  Select [1-{}]: ", sessions.len() + 1);

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let choice: usize = input.trim().parse().unwrap_or(0);

    if choice >= 1 && choice <= sessions.len() {
        let session = &sessions[choice - 1];
        Ok((session.name.clone(), session.worker_count as u16, true))
    } else {
        eprint!("  Session name: ");
        let mut name = String::new();
        std::io::stdin().read_line(&mut name)?;
        let name = name.trim().to_string();
        if name.is_empty() {
            anyhow::bail!("Session name cannot be empty");
        }
        Ok((name, default_workers, false))
    }
}

async fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        // Update orchestrate snapshot for UI rendering
        if let Some(engine) = app.orchestrate.clone() {
            app.orchestrate_snapshot = Some(engine.all_status().await);
        }

        terminal.draw(|frame| draw(frame, app))?;

        if event::poll(std::time::Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    match handle_key(app, key) {
                        InputResult::Quit => break,
                        InputResult::Continue => {}
                    }
                }
                Event::Mouse(mouse) => {
                    handle_mouse(app, mouse);
                }
                Event::Resize(w, h) => {
                    app.resize_panes(w, h);
                }
                _ => {}
            }
        }

        // Poll for pending tasks and inject into idle Worker PTYs
        // Clone engine to avoid holding an immutable borrow on app while we need mutable access
        if let Some(engine) = app.orchestrate.clone() {
            // Collect (pane_idx, ticket) pairs first, then inject
            let mut injections: Vec<(usize, String)> = Vec::new();
            if let Some(ref snapshot) = app.orchestrate_snapshot {
                for ws in snapshot {
                    if ws.status == legion_core::WorkerTaskStatus::Pending {
                        let pane_idx = ws.worker_id as usize;
                        if let Some(parser) = app.parser_at(pane_idx) {
                            if pty::is_pty_idle(parser) {
                                if let Some(ticket) = engine.take_pending(ws.worker_id).await {
                                    injections.push((pane_idx, ticket));
                                }
                            }
                        }
                    }
                }
            }
            for (pane_idx, ticket) in injections {
                app.write_to_pane(pane_idx, b"\x15");
                app.write_to_pane(pane_idx, ticket.as_bytes());
                app.write_to_pane(pane_idx, b"\r");
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}
