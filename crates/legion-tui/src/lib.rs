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
    // Setup terminal with mouse support for divider dragging
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and load providers from DB
    let mut app = App::new();
    app.load_from_db();

    // Cache terminal size
    let size = terminal.size()?;
    app.term_size = (size.width, size.height);

    // Calculate PTY sizes based on layout
    let content_height = size.height.saturating_sub(2); // header + footer

    // Leader pane: leader_ratio% width
    let leader_width = (size.width as u32 * app.leader_ratio as u32 / 100) as u16;
    let leader_pty_rows = content_height.saturating_sub(2);
    let leader_pty_cols = leader_width.saturating_sub(2);

    // Worker panes: remaining width minus 1 for divider, height divided equally
    let worker_width = size.width.saturating_sub(leader_width).saturating_sub(1);
    let worker_height = content_height / worker_count;
    let worker_pty_rows = worker_height.saturating_sub(2);
    let worker_pty_cols = worker_width.saturating_sub(2);

    // Generate system prompts for Leader and Workers
    let leader_prompt = claudemd::leader_instructions(worker_count);
    let worker_prompts: Vec<String> = (1..=worker_count)
        .map(|id| claudemd::worker_instructions(id))
        .collect();

    // Port assignments:
    // Leader: proxy = base_port, control = base_port + 1000
    // Worker i: proxy = base_port + i + 1, control = base_port + 1000 + i + 1
    let orchestrate_port = base_port + 2000;
    let leader_proxy = base_port;
    let leader_control = base_port + 1000;
    // Leader: normal interactive mode (no skip permissions)
    // Workers: auto-trust mode (skip permissions)
    app.add_pane(leader_pty_rows, leader_pty_cols, leader_proxy, leader_control, "Leader".into(), false, None, Some(orchestrate_port), Some(&leader_prompt), None, false);

    for i in 0..worker_count {
        let proxy = base_port + i + 1;
        let control = base_port + 1000 + i + 1;
        let label = format!("Worker {}", i + 1);
        app.add_pane(worker_pty_rows, worker_pty_cols, proxy, control, label, true, Some(i + 1), Some(orchestrate_port), Some(&worker_prompts[i as usize]), None, false);
    }

    // Start orchestration engine + API
    let engine = legion_core::OrchestrateEngine::new(worker_count);
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

    // Force exit — background tokio tasks (proxy/control/orchestrate servers)
    // run infinite accept loops that would otherwise delay shutdown.
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
