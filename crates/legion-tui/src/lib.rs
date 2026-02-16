//! Legion TUI - Embedded terminal interface with provider/model switching

pub mod app;
pub mod claudemd;
pub mod diff;
pub mod input;
pub mod pty;
pub mod sdk;
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

use legion_core::proxy::{ProxyControlApi, ProxyServer};

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

    // Setup terminal with mouse support for divider dragging
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and load providers from DB
    let mut app = App::new();
    app.load_from_db();
    app.project_path = Some(project_path);
    app.base_port = base_port;
    app.requested_workers = worker_count;

    // Cache terminal size
    let size = terminal.size()?;
    app.term_size = (size.width, size.height);

    // Check for default session — auto-resume if exists
    let default_session = if let Ok(repo) = legion_db::open_db() {
        let pp = app.project_path.as_ref().unwrap().to_string_lossy().to_string();
        repo.get_default_squad_session(&pp).ok().flatten()
    } else {
        None
    };

    if let Some(default_sess) = default_session {
        // Auto-resume default session
        let workers = default_sess.worker_count as u16;
        match app.start_session(&default_sess.name, workers, true, true) {
            Ok(()) => {
                tracing::info!("Auto-resumed default session: {}", default_sess.name);
                app.mode = app::AppMode::Normal;
            }
            Err(e) => {
                tracing::error!("Failed to resume default session: {}", e);
                app.mode = app::AppMode::Popup(app::PopupMenu::NewSessionInput);
                app.session_name_input = app.default_session_name_for_default();
            }
        }
    } else {
        // No default session — first-time setup
        app.mode = app::AppMode::Popup(app::PopupMenu::NewSessionInput);
        app.session_name_input = app.default_session_name_for_default();
        app.creating_default_session = true;
    }

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

async fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    let mut read_leader_status_tick: u32 = 0;
    loop {
        // Start orchestration API if a session was just spawned
        if let Some(orch_port) = app.pending_orchestrate_port.take() {
            if let Some(engine) = app.orchestrate.clone() {
                let orch_api = legion_core::OrchestrateApi::new(engine, orch_port);
                tokio::spawn(async move {
                    if let Err(e) = orch_api.start_with_signal(None).await {
                        tracing::error!("Orchestrate API error on port {}: {}", orch_port, e);
                    }
                });
            }
        }

        // Check if any --continue panes failed and need respawn
        app.check_continue_fallback();

        // Handle pending add worker
        if app.pending_add_worker {
            app.pending_add_worker = false;
            handle_add_worker(app).await;
        }

        // Sync max iterations to engine
        if app.pending_sync_max_iterations {
            app.pending_sync_max_iterations = false;
            if let Some(ref engine) = app.orchestrate {
                engine.set_default_max_iterations(app.default_max_iterations).await;
            }
        }

        // Handle pending remove worker
        if let Some((pane_index, strategy)) = app.pending_remove_worker.take() {
            match app.remove_single_worker(pane_index, &strategy) {
                Ok(()) => tracing::info!("Worker removed successfully"),
                Err(e) => tracing::error!("Failed to remove worker: {}", e),
            }
        }

        // Periodically read leader context status (every ~2s = 40 ticks at 50ms)
        read_leader_status_tick += 1;
        if read_leader_status_tick >= 40 {
            read_leader_status_tick = 0;
            read_leader_status(app);
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

        // --- SDK dispatch: idle workers pull from queue ---
        if let Some(engine) = app.orchestrate.clone() {
            // Update ticket snapshots for UI
            app.ticket_snapshot = Some(engine.all_tickets().await);
            app.queue_stats = Some(engine.queue_stats().await);

            // Auto-select first ticket if current selection is invalid
            if let Some(ref tickets) = app.ticket_snapshot {
                if !tickets.is_empty() && !tickets.iter().any(|t| t.id == app.board_selected) {
                    app.board_selected = tickets[0].id;
                }
            }

            let wc = engine.worker_count().await;

            for wi in 1..=wc as usize {
                if wi >= app.panes.len() { break; }

                // Drain new SDK entries
                let drained = app.panes[wi].sdk_task.as_mut()
                    .map(|sdk| sdk.drain_entries())
                    .unwrap_or_default();
                app.panes[wi].sdk_entries.extend(drained);

                // Check if SDK finished — collect info before any mutation
                let finished_info = {
                    let pane = &app.panes[wi];
                    if let Some(ref sdk) = pane.sdk_task {
                        if sdk.is_finished() {
                            let result_text = sdk.result_text().unwrap_or_default();
                            let ticket_id = pane.current_ticket_id.unwrap_or(0);
                            Some((ticket_id, result_text))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };

                if let Some((ticket_id, result_text)) = finished_info {
                    let promise_found = crate::sdk::detect_promise(&result_text);
                    let mut should_cache_diff = false;

                    if promise_found {
                        // Success — report to engine
                        let summary = Some(crate::sdk::extract_feedback(&result_text));
                        engine.report_iteration(ticket_id, true, summary).await;
                        tracing::info!("Worker {} ticket {} completed (promise found)", wi, ticket_id);
                        should_cache_diff = true;
                    } else {
                        // Failed — check if retry needed
                        let feedback = crate::sdk::extract_feedback(&result_text);
                        let should_retry = engine.report_iteration(ticket_id, false, Some(feedback.clone())).await;

                        if should_retry {
                            // Get updated ticket info for retry
                            if let Some(ts) = engine.worker_ticket(wi as u16).await {
                                tracing::info!(
                                    "Worker {} ticket {} retrying (iter {})",
                                    wi, ticket_id, ts.iteration
                                );
                                let prompt = ts.prompt.clone();
                                let team_mode = ts.team_mode.clone();
                                let iteration = ts.iteration;
                                let title = ts.title.clone();
                                let context = ts.context.clone();
                                let criteria = ts.criteria.clone();
                                // Clean up old SDK
                                app.panes[wi].sdk_task = None;
                                // Start new iteration
                                app.start_sdk_task(wi, ticket_id, &prompt, &team_mode, iteration, Some(&feedback),
                                    &title, context.as_deref(), criteria.as_deref());
                                continue;
                            }
                        } else {
                            tracing::warn!("Worker {} ticket {} failed (max iterations)", wi, ticket_id);
                            should_cache_diff = true;
                        }
                    }

                    // Cache diff for Done/Error tickets
                    if should_cache_diff {
                        if let (Some(project_path), Some(session)) = (&app.project_path, &app.current_session) {
                            let wt_path = crate::worktree::pane_worktree_path(
                                project_path, &session.name, &format!("Worker {}", wi),
                            );
                            let leader_ref = crate::diff::get_leader_ref(
                                project_path, &session.name, session.is_default,
                            );
                            let session_name = session.name.clone();
                            let db = engine.db().cloned();
                            tokio::task::spawn_blocking(move || {
                                match crate::diff::get_worktree_diff(&wt_path, &leader_ref, false) {
                                    Ok(data) => {
                                        let file_summary: Vec<legion_db::FileDiffSummary> = data.files.iter().map(|f| {
                                            legion_db::FileDiffSummary {
                                                path: f.path.clone(),
                                                status: f.status.clone(),
                                                additions: f.additions,
                                                deletions: f.deletions,
                                            }
                                        }).collect();
                                        let summary_json = serde_json::to_string(&file_summary).unwrap_or_default();
                                        let now = std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_secs() as i64;
                                        if let Some(db) = db {
                                            if let Ok(db) = db.lock() {
                                                if let Err(e) = db.save_ticket_diff(
                                                    ticket_id as i64, &session_name,
                                                    &data.raw_diff, &summary_json, now,
                                                ) {
                                                    tracing::warn!("Failed to cache diff for ticket {}: {}", ticket_id, e);
                                                }
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        tracing::warn!("Failed to get diff for ticket {}: {}", ticket_id, e);
                                    }
                                }
                            });
                        }
                    }

                    // Clean up finished SDK (log buffer stays in app.ticket_logs keyed by ticket_id)
                    app.panes[wi].sdk_task = None;
                    app.panes[wi].current_ticket_id = None;
                    app.panes[wi].sdk_log_buffer = None;
                }

                // If worker is idle and no SDK running, try to take next ticket
                if app.panes[wi].sdk_task.is_none() {
                    if let Some(ts) = engine.take_next(wi as u16).await {
                        tracing::info!("Worker {} taking ticket {}", wi, ts.id);
                        app.start_sdk_task(wi, ts.id, &ts.prompt, &ts.team_mode, 1, None,
                            ts.title.as_str(), ts.context.as_deref(), ts.criteria.as_deref());
                    }
                }
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}

/// Spawn a new worker: create proxy+control servers, worktree, add pane (SDK-based)
async fn handle_add_worker(app: &mut App) {
    let worker_id = app.next_worker_id;
    app.next_worker_id += 1;

    let label = format!("Worker {}", worker_id);
    let base_port = app.base_port;
    let proxy_port = base_port + worker_id;
    let control_port = base_port + 1000 + worker_id;
    tracing::info!("Adding worker '{}': proxy={}, control={}", label, proxy_port, control_port);

    // Create and start proxy server
    let proxy = ProxyServer::new(proxy_port);

    // Apply current provider config to the new proxy
    if let Some(provider) = app.current_provider.and_then(|i| app.providers.get(i)) {
        if provider.id != "__default__" {
            let config = legion_core::ProxyConfig {
                target_url: Some(provider.base_url.clone()),
                api_key: provider.api_key.clone(),
                api_format: Some(provider.api_format.clone()),
                model: app.current_model.clone(),
            };
            proxy.update_config(config).await;
        }
    }

    let (proxy_tx, proxy_rx) = tokio::sync::oneshot::channel();
    let proxy_config_ref = proxy.config_ref();
    tokio::spawn(async move {
        if let Err(e) = proxy.start_with_signal(Some(proxy_tx)).await {
            tracing::error!("Proxy error on port {}: {}", proxy_port, e);
        }
    });

    let (control_tx, control_rx) = tokio::sync::oneshot::channel();
    let control_api = ProxyControlApi::new(proxy_config_ref, control_port);
    tokio::spawn(async move {
        if let Err(e) = control_api.start_with_signal(Some(control_tx)).await {
            tracing::error!("Control API error on port {}: {}", control_port, e);
        }
    });

    // Wait briefly for servers to be ready
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        proxy_rx.await.ok();
        control_rx.await.ok();
    }).await;

    // Create worktree for the new worker
    if let (Some(ref project_path), Some(ref session)) = (&app.project_path, &app.current_session) {
        if let Err(e) = worktree::create_worktree(project_path, &session.name, &label) {
            tracing::error!("Failed to create worktree for '{}': {}", label, e);
            return;
        }
    }

    // Workers: create pane without PTY (SDK will be used when ticket assigned)
    // Check for saved per-pane config to restore provider/model on restart
    let (pane_provider, pane_model) = if let Some((saved_pid, saved_model)) = app.get_saved_pane_config(&label) {
        let provider_idx = app.providers.iter().position(|p| p.id == *saved_pid);
        if provider_idx.is_some() {
            (provider_idx, saved_model.clone())
        } else {
            (app.current_provider, app.current_model.clone())
        }
    } else {
        (app.current_provider, app.current_model.clone())
    };
    app.panes.push(app::Pane {
        pty: None,
        proxy_port,
        control_port,
        label: label.clone(),
        current_provider: pane_provider,
        current_model: pane_model,
        spawned_with_continue: false,
        sdk_task: None,
        sdk_parser: None,
        sdk_entries: Vec::new(),
        current_ticket_id: None,
        sdk_log_buffer: None,
    });

    // Resize all panes to accommodate the new worker
    app.apply_resize();

    tracing::info!("Worker '{}' added successfully", label);
}

/// Read leader context status from the statusLine hook output file.
fn read_leader_status(app: &mut App) {
    let path = std::path::Path::new(crate::pty::LEADER_STATUS_PATH);
    if !path.exists() {
        return;
    }
    if let Ok(content) = std::fs::read_to_string(path) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
            // Context percentage
            if let Some(pct) = json.get("context_window")
                .and_then(|cw| cw.get("used_percentage"))
                .and_then(|v| v.as_f64())
            {
                app.leader_context_pct = Some(pct.round() as u8);
            }
            // Git branch from cwd
            if let Some(cwd) = json.get("cwd").and_then(|v| v.as_str()) {
                if let Ok(output) = std::process::Command::new("git")
                    .args(["rev-parse", "--abbrev-ref", "HEAD"])
                    .current_dir(cwd)
                    .output()
                {
                    if output.status.success() {
                        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
                        if !branch.is_empty() {
                            app.leader_git_branch = Some(branch);
                        }
                    }
                }
            }
        }
    }
}
