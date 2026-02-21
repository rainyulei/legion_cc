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
    event::{self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use app::{AppMode, PopupMenu};

use legion_core::proxy::{ProxyControlApi, ProxyServer};

use app::App;
use input::{handle_key, handle_mouse, InputResult};
use ui::draw;

/// Run the full TUI with a single embedded Claude Code
pub async fn run(proxy_port: u16, control_port: u16) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
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
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableBracketedPaste);
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
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and load providers from DB
    let mut app = App::new();
    app.load_from_db();
    app.project_path = Some(project_path);
    app.project_db_path = app.project_path.clone();
    app.base_port = base_port;
    app.requested_workers = worker_count;

    // Cache terminal size
    let size = terminal.size()?;
    app.term_size = (size.width, size.height);

    // Detect current branch
    if let Some(ref project_path) = app.project_path {
        app.detected_branch = crate::worktree::current_branch(project_path);
        app.detected_commit = crate::worktree::current_commit(project_path);
    }

    // Always show session list on startup — let user choose
    app.load_session_list();

    if app.session_list.iter().any(|s| s.status == "active") {
        // Has existing sessions — show session list, pre-select last used (or first active)
        let last_used_idx = app.session_list.iter()
            .enumerate()
            .filter(|(_, s)| s.status == "active")
            .max_by_key(|(_, s)| s.last_active_at.unwrap_or(s.created_at))
            .map(|(i, _)| i)
            .unwrap_or(0);
        app.session_list_index = last_used_idx;
        app.mode = app::AppMode::Popup(app::PopupMenu::SessionList);
    } else {
        // No sessions — go to new session input with branch auto-fill
        if let Some(ref branch) = app.detected_branch {
            app.session_name_input = crate::worktree::sanitize_branch_name(branch);
        } else {
            app.session_name_input = app.default_session_name_for_default();
        }
        // Load branch list for the new session popup
        if let Some(ref project_path) = app.project_path {
            app.branch_list = crate::worktree::list_local_branches(project_path);
        }
        app.new_session_branch_index = app.detected_branch.as_ref()
            .and_then(|db| app.branch_list.iter().position(|b| b == db))
            .unwrap_or(0);
        app.new_session_branch_focused = true;
        app.mode = app::AppMode::Popup(app::PopupMenu::NewSessionInput);
        app.creating_default_session = true;
    }

    // Main event loop
    let result = run_event_loop(&mut terminal, &mut app).await;

    // Kill child processes and restore terminal
    app.kill_all();
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture, DisableBracketedPaste);
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
            // Shutdown previous orchestrate API if running (prevents port conflict)
            if let Some(tx) = app.orchestrate_shutdown.take() {
                let _ = tx.send(());
                // Brief yield to let old server release the port
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            if let Some(engine) = app.orchestrate.clone() {
                let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
                app.orchestrate_shutdown = Some(shutdown_tx);
                let orch_api = legion_core::OrchestrateApi::new(engine, orch_port);
                tokio::spawn(async move {
                    if let Err(e) = orch_api.start_with_shutdown(None, Some(shutdown_rx)).await {
                        tracing::error!("Orchestrate API error on port {}: {}", orch_port, e);
                    }
                });
            }
        }

        // Start proxy servers for workers queued by start_session
        if !app.pending_worker_proxies.is_empty() {
            let proxies = std::mem::take(&mut app.pending_worker_proxies);
            for (proxy_port, control_port, label) in &proxies {
                start_worker_proxy(*proxy_port, *control_port, label).await;
            }
            // Send proxy configs to all worker proxies
            send_proxy_configs_sync(app).await;
        }

        // Check if any --continue panes failed and need respawn
        app.check_continue_fallback();

        // Handle pending set worker count (add/remove workers to reach target)
        if let Some(target) = app.pending_set_worker_count.take() {
            let current = app.worker_count() as u16;
            if target > current {
                for _ in 0..(target - current) {
                    handle_add_worker(app).await;
                }
            } else if target < current {
                // Remove from the end, merging worktrees
                for _ in 0..(current - target) {
                    let pane_index = app.panes.len() - 1; // last worker pane
                    match app.remove_single_worker(pane_index, "merge") {
                        Ok(()) => tracing::info!("Worker removed (scaling down)"),
                        Err(e) => {
                            tracing::error!("Failed to remove worker: {}", e);
                            break;
                        }
                    }
                }
            }
            app.persist_worker_count();
            // Sync engine worker_count so scheduling loop covers all workers
            if let Some(ref engine) = app.orchestrate {
                engine.set_worker_count(app.worker_count() as u16).await;
            }
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
                Ok(()) => {
                    app.persist_worker_count();
                    if let Some(ref engine) = app.orchestrate {
                        engine.set_worker_count(app.worker_count() as u16).await;
                    }
                    tracing::info!("Worker removed successfully");
                }
                Err(e) => tracing::error!("Failed to remove worker: {}", e),
            }
        }

        // Periodically read leader context status (every ~2s = 40 ticks at 50ms)
        read_leader_status_tick += 1;
        if read_leader_status_tick >= 40 {
            read_leader_status_tick = 0;
            read_leader_status(app);
        }

        // Periodic branch check (every 5s)
        if app.mode == app::AppMode::Normal {
            let should_check = app.last_branch_check
                .map(|t| t.elapsed().as_secs() >= 5)
                .unwrap_or(true);
            if should_check {
                app.last_branch_check = Some(std::time::Instant::now());
                if let (Some(ref session), Some(ref project_path)) = (&app.current_session, &app.project_path) {
                    if let Some(ref base_branch) = session.base_branch {
                        if let Some(current) = crate::worktree::current_branch(project_path) {
                            if &current != base_branch {
                                app.branch_changed_to = Some(current);
                                app.recovery_choice = 0;
                                app.mode = app::AppMode::Popup(app::PopupMenu::BranchChanged);
                            }
                        }
                    }
                }
            }
        }

        // Check Copilot auth channel
        if let Some(mut rx) = app.copilot_auth_rx.take() {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    app::CopilotAuthMsg::DeviceCode { user_code, verification_uri } => {
                        app.copilot_user_code = Some(user_code);
                        app.copilot_verification_uri = Some(verification_uri);
                        app.copilot_auth_status = app::CopilotAuthStatus::WaitingForAuth;
                    }
                    app::CopilotAuthMsg::Authorized => {
                        app.copilot_auth_status = app::CopilotAuthStatus::Exchanging;
                    }
                    app::CopilotAuthMsg::SetupComplete { models } => {
                        app.copilot_models_result = Some(models);
                        app.copilot_auth_status = app::CopilotAuthStatus::Success;
                        // Reload providers from DB
                        app.load_from_db();
                    }
                    app::CopilotAuthMsg::Error(e) => {
                        app.copilot_auth_error = Some(e);
                        app.copilot_auth_status = app::CopilotAuthStatus::Error;
                    }
                }
            }
            app.copilot_auth_rx = Some(rx);
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
                Event::Paste(data) => {
                    // Route paste to the appropriate input buffer based on mode
                    match app.mode {
                        AppMode::Popup(PopupMenu::ProviderApiKeyInput) => {
                            app.api_key_input.push_str(&data);
                        }
                        AppMode::Popup(PopupMenu::NewSessionInput) => {
                            app.session_name_input.push_str(&data);
                        }
                        AppMode::Popup(PopupMenu::RetryForm) => {
                            let focus = app.retry_form_focus as usize;
                            if focus < app.retry_form_fields.len() {
                                app.retry_form_fields[focus].push_str(&data);
                            }
                        }
                        _ => {
                            // Normal mode: send to PTY with bracketed paste
                            let mut paste_bytes = Vec::with_capacity(data.len() + 12);
                            paste_bytes.extend_from_slice(b"\x1b[200~");
                            paste_bytes.extend_from_slice(data.as_bytes());
                            paste_bytes.extend_from_slice(b"\x1b[201~");
                            app.write_to_pty(&paste_bytes);
                        }
                    }
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
            // Prefer highest-priority status: Working > Queued > Done > Error
            if let Some(ref tickets) = app.ticket_snapshot {
                if !tickets.is_empty() && !tickets.iter().any(|t| t.id == app.board_selected) {
                    use legion_core::orchestrate::engine::TicketStatus;
                    let first_by_priority = tickets.iter().find(|t| t.status == TicketStatus::Working)
                        .or_else(|| tickets.iter().find(|t| t.status == TicketStatus::Queued))
                        .or_else(|| tickets.iter().find(|t| t.status == TicketStatus::Done))
                        .or_else(|| tickets.iter().find(|t| t.status == TicketStatus::Error))
                        .unwrap_or(&tickets[0]);
                    app.board_selected = first_by_priority.id;
                }
            }

            let wc = engine.worker_count().await;

            for wi in 1..=wc as usize {
                if wi >= app.panes.len() { break; }

                // Drain new SDK entries and extract TeamActivity
                let drained = app.panes[wi].sdk_task.as_mut()
                    .map(|sdk| sdk.drain_entries())
                    .unwrap_or_default();
                if !drained.is_empty() {
                    app.panes[wi].last_sdk_activity = Some(std::time::Instant::now());
                }
                // Extract TeamActivity from Task tool entries
                if let Some(ticket_id) = app.panes[wi].current_ticket_id {
                    let elapsed = app.panes[wi].last_sdk_activity
                        .map(|t| t.elapsed().as_secs())
                        .unwrap_or(0);
                    let mut last_was_task = false;
                    for entry in &drained {
                        match entry {
                            crate::sdk::ProgressEntry::ToolUse { tool_name, summary } => {
                                if tool_name == "Task" {
                                    last_was_task = true;
                                    if let Some((role, desc)) = crate::sdk::parse_team_activity(summary) {
                                        let activities = app.ticket_team_activities.entry(ticket_id).or_default();
                                        activities.push(crate::sdk::TeamActivity {
                                            role,
                                            description: desc,
                                            status: crate::sdk::TeamActivityStatus::Running,
                                            elapsed_secs: elapsed,
                                        });
                                    }
                                } else {
                                    last_was_task = false;
                                }
                            }
                            crate::sdk::ProgressEntry::ToolResult { summary, is_error } if last_was_task => {
                                last_was_task = false;
                                // Update the last Running TeamActivity for this ticket
                                if let Some(activities) = app.ticket_team_activities.get_mut(&ticket_id) {
                                    if let Some(last) = activities.iter_mut().rev()
                                        .find(|a| a.status == crate::sdk::TeamActivityStatus::Running)
                                    {
                                        last.status = if *is_error {
                                            let msg = if summary.is_empty() { "Error".to_string() } else {
                                                summary.chars().take(100).collect()
                                            };
                                            crate::sdk::TeamActivityStatus::Error(msg)
                                        } else {
                                            let msg = summary.chars().take(100).collect();
                                            crate::sdk::TeamActivityStatus::Done(msg)
                                        };
                                    }
                                }
                            }
                            _ => {
                                last_was_task = false;
                            }
                        }
                    }
                }
                app.panes[wi].sdk_entries.extend(drained);

                // Timeout check: 5 min no SDK output → kill and report error
                if app.panes[wi].sdk_task.is_some()
                    && !app.panes[wi].sdk_task.as_ref().unwrap().is_finished()
                {
                    if let Some(last) = app.panes[wi].last_sdk_activity {
                        if last.elapsed().as_secs() >= app::WORKER_TIMEOUT_SECS {
                            let ticket_id = app.panes[wi].current_ticket_id.unwrap_or(0);
                            tracing::warn!(
                                "Worker {} ticket {} timed out (no output for {}s)",
                                wi, ticket_id, app::WORKER_TIMEOUT_SECS
                            );
                            if let Some(ref mut sdk) = app.panes[wi].sdk_task {
                                sdk.kill();
                            }
                            engine.report_iteration(ticket_id, false, Some("Worker timed out: no output for 5 minutes".into())).await;
                            engine.force_error(ticket_id, "Timed out: no output for 5 minutes").await;
                            app.panes[wi].sdk_task = None;
                            app.panes[wi].current_ticket_id = None;
                            app.panes[wi].sdk_log_buffer = None;
                            app.panes[wi].last_sdk_activity = None;
                            continue;
                        }
                    }
                }

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
                                let structure_plan = ts.structure_plan.clone();
                                // Clean up old SDK
                                app.panes[wi].sdk_task = None;
                                // Start new iteration
                                app.start_sdk_task(wi, ticket_id, &prompt, &team_mode, iteration, Some(&feedback),
                                    &title, context.as_deref(), criteria.as_deref(), structure_plan.as_deref());
                                continue;
                            }
                        } else {
                            tracing::warn!("Worker {} ticket {} failed (max iterations)", wi, ticket_id);
                            should_cache_diff = true;
                        }
                    }

                    // Cache diff for Done/Error tickets (synchronous to ensure it completes
                    // before worktrees are potentially removed on session stop)
                    if should_cache_diff {
                        // Get base_commit from ticket for accurate diff
                        let base_commit = {
                            let tickets = engine.all_tickets().await;
                            tickets.iter().find(|t| t.id == ticket_id).and_then(|t| t.base_commit.clone())
                        };
                        if let (Some(project_path), Some(session)) = (&app.project_path, &app.current_session) {
                            let wt_path = crate::worktree::pane_worktree_path(
                                project_path, &session.name, &format!("Worker {}", wi),
                            );
                            let leader_ref = crate::diff::get_leader_ref(
                                project_path, &session.name, session.is_default,
                            );
                            let session_name = session.name.clone();
                            let db = engine.db().cloned();
                            let cache_handle = tokio::task::spawn_blocking(move || {
                                // Use base_commit for direct diff if available, otherwise fall back to merge-base
                                let diff_result = if let Some(ref base) = base_commit {
                                    crate::diff::get_worktree_diff_from_commit(&wt_path, base)
                                } else {
                                    crate::diff::get_worktree_diff(&wt_path, &leader_ref, true)
                                };
                                match diff_result {
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
                            // Wait for diff caching to complete (with timeout)
                            let _ = tokio::time::timeout(
                                std::time::Duration::from_secs(10),
                                cache_handle,
                            ).await;
                        }
                    }

                    // Auto-merge: merge worker branch into leader (only for Done tickets)
                    if promise_found {
                        if let (Some(project_path), Some(session)) = (&app.project_path, &app.current_session) {
                            let worker_label = format!("Worker {}", wi);

                            // Fallback: if worker forgot to commit, auto-commit before merge
                            let worker_dir = crate::worktree::pane_worktree_path(
                                project_path, &session.name, &worker_label,
                            );
                            if worker_dir.exists() {
                                let status_out = std::process::Command::new("git")
                                    .args(["status", "--porcelain"])
                                    .current_dir(&worker_dir)
                                    .output();
                                if let Ok(out) = status_out {
                                    let status_str = String::from_utf8_lossy(&out.stdout);
                                    if !status_str.trim().is_empty() {
                                        tracing::warn!("{} has uncommitted changes, auto-committing", worker_label);
                                        let _ = std::process::Command::new("git")
                                            .args(["add", "-A"])
                                            .current_dir(&worker_dir)
                                            .output();
                                        let _ = std::process::Command::new("git")
                                            .args(["commit", "-m", "feat: auto-commit worker changes (worker forgot to commit)"])
                                            .current_dir(&worker_dir)
                                            .output();
                                    }
                                }
                            }

                            // Smart merge: try clean merge first, then AI resolution on conflict
                            match crate::worktree::merge_worker_no_abort(
                                project_path, &session.name, &worker_label, session.is_default,
                            ) {
                                Ok(crate::worktree::MergeResult::Clean) => {
                                    tracing::info!("Auto-merged {} into leader (clean)", worker_label);
                                    engine.set_merge_status(ticket_id, legion_core::MergeStatus::Merged).await;
                                }
                                Ok(crate::worktree::MergeResult::Conflict(conflict_ctx)) => {
                                    tracing::warn!("Merge conflict for {}, launching AI resolver", worker_label);

                                    // Gather done ticket context for the resolution prompt
                                    let all_tickets = engine.all_tickets().await;
                                    let done_tickets: Vec<(String, Option<String>)> = all_tickets.iter()
                                        .filter(|t| t.status == legion_core::TicketStatus::Done && t.id != ticket_id)
                                        .filter(|t| t.merge_status == legion_core::MergeStatus::Merged)
                                        .map(|t| (t.title.clone(), t.summary.clone()))
                                        .collect();

                                    let ticket_title = all_tickets.iter()
                                        .find(|t| t.id == ticket_id)
                                        .map(|t| t.title.clone())
                                        .unwrap_or_default();
                                    let ticket_summary = all_tickets.iter()
                                        .find(|t| t.id == ticket_id)
                                        .and_then(|t| t.summary.clone());

                                    let resolve_prompt = crate::claudemd::conflict_resolve_prompt(
                                        &conflict_ctx, &ticket_title, ticket_summary.as_deref(), &done_tickets,
                                    );

                                    let leader_dir = if session.is_default {
                                        project_path.to_path_buf()
                                    } else {
                                        crate::worktree::pane_worktree_path(project_path, &session.name, "Leader")
                                    };

                                    // Spawn SDK process to resolve conflicts (3 min timeout)
                                    let (term_w, term_h) = app.term_size;
                                    let resolve_parser = std::sync::Arc::new(std::sync::Mutex::new(
                                        vt100::Parser::new(term_h.saturating_sub(4), term_w.saturating_sub(4), 1000)
                                    ));
                                    let use_proxy = app.pane_uses_proxy("Leader");
                                    let leader_proxy_port = app.panes[0].proxy_port;

                                    match crate::sdk::SdkHandle::spawn(
                                        &leader_dir, &resolve_prompt, resolve_parser,
                                        use_proxy, leader_proxy_port,
                                        Some("You are a merge conflict resolver. Your job is to resolve git merge conflicts intelligently, preserving both sides' intent. Work in the current directory. Do not abort the merge."),
                                        1, None, None,
                                    ) {
                                        Ok(mut resolver) => {
                                            // Wait for resolution with 3 minute timeout
                                            let resolve_result = tokio::time::timeout(
                                                std::time::Duration::from_secs(180),
                                                async {
                                                    loop {
                                                        if resolver.is_finished() {
                                                            return resolver.result_text().unwrap_or_default();
                                                        }
                                                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                                                    }
                                                }
                                            ).await;

                                            match resolve_result {
                                                Ok(result_text) if crate::sdk::detect_promise(&result_text) => {
                                                    // AI resolved successfully — finalize merge
                                                    match crate::worktree::finalize_resolved_merge(
                                                        &leader_dir, &worker_label, conflict_ctx.did_stash,
                                                    ) {
                                                        Ok(()) => {
                                                            // Run build check
                                                            match crate::worktree::run_build_check(&leader_dir) {
                                                                Ok(true) => {
                                                                    tracing::info!("AI-resolved merge for {} passed build check", worker_label);
                                                                    engine.set_merge_status(ticket_id, legion_core::MergeStatus::Merged).await;
                                                                }
                                                                Ok(false) => {
                                                                    tracing::warn!("AI-resolved merge for {} failed build check, aborting", worker_label);
                                                                    // Undo the merge commit
                                                                    let _ = std::process::Command::new("git")
                                                                        .args(["reset", "--hard", "HEAD~1"])
                                                                        .current_dir(&leader_dir)
                                                                        .output();
                                                                    if conflict_ctx.did_stash {
                                                                        let _ = std::process::Command::new("git")
                                                                            .args(["stash", "pop"])
                                                                            .current_dir(&leader_dir)
                                                                            .output();
                                                                    }
                                                                    engine.set_merge_status(ticket_id, legion_core::MergeStatus::Conflict).await;
                                                                }
                                                                Err(e) => {
                                                                    tracing::warn!("Build check failed to run for {}: {}", worker_label, e);
                                                                    // Don't abort — build check failure to run is not a merge failure
                                                                    engine.set_merge_status(ticket_id, legion_core::MergeStatus::Merged).await;
                                                                }
                                                            }
                                                        }
                                                        Err(e) => {
                                                            tracing::warn!("Failed to finalize AI-resolved merge for {}: {}", worker_label, e);
                                                            let _ = crate::worktree::abort_merge_and_restore(&leader_dir, conflict_ctx.did_stash);
                                                            engine.set_merge_status(ticket_id, legion_core::MergeStatus::Conflict).await;
                                                        }
                                                    }
                                                }
                                                _ => {
                                                    // AI failed or timed out — abort merge
                                                    tracing::warn!("AI conflict resolution failed/timed out for {}", worker_label);
                                                    resolver.kill();
                                                    let _ = crate::worktree::abort_merge_and_restore(&leader_dir, conflict_ctx.did_stash);
                                                    engine.set_merge_status(ticket_id, legion_core::MergeStatus::Conflict).await;
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::error!("Failed to spawn AI conflict resolver for {}: {}", worker_label, e);
                                            let _ = crate::worktree::abort_merge_and_restore(&leader_dir, conflict_ctx.did_stash);
                                            engine.set_merge_status(ticket_id, legion_core::MergeStatus::Conflict).await;
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!("Auto-merge failed for {}: {}", worker_label, e);
                                    engine.set_merge_status(ticket_id, legion_core::MergeStatus::Conflict).await;
                                }
                            }
                        }
                    }

                    // Clean up finished SDK (log buffer stays in app.ticket_logs keyed by ticket_id)
                    app.panes[wi].sdk_task = None;
                    app.panes[wi].current_ticket_id = None;
                    app.panes[wi].sdk_log_buffer = None;
                    app.panes[wi].last_sdk_activity = None;
                }

                // If worker is idle and no SDK running, try to take next ticket
                if app.panes[wi].sdk_task.is_none() {
                    if let Some(ts) = engine.take_next(wi as u16).await {
                        tracing::info!("Worker {} taking ticket {}", wi, ts.id);

                        if let (Some(project_path), Some(session)) = (&app.project_path, &app.current_session) {
                            let worker_label = format!("Worker {}", wi);
                            let wt_path = crate::worktree::pane_worktree_path(
                                project_path, &session.name, &worker_label,
                            );

                            // Rebase worker worktree to leader's latest before starting
                            if let Err(e) = crate::worktree::rebase_worker_from_leader(
                                project_path, &session.name, &worker_label, session.is_default,
                            ) {
                                tracing::warn!("Worker {} rebase failed: {}", wi, e);
                            }

                            // Capture base commit after rebase (reflects leader's latest)
                            if let Ok(output) = std::process::Command::new("git")
                                .args(["rev-parse", "HEAD"])
                                .current_dir(&wt_path)
                                .output()
                            {
                                if output.status.success() {
                                    let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
                                    engine.set_base_commit(ts.id, commit).await;
                                }
                            }
                        }

                        app.start_sdk_task(wi, ts.id, &ts.prompt, &ts.team_mode, 1, None,
                            ts.title.as_str(), ts.context.as_deref(), ts.criteria.as_deref(), ts.structure_plan.as_deref());
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

/// Create and start proxy + control API servers for a worker pane.
async fn start_worker_proxy(proxy_port: u16, control_port: u16, label: &str) {
    let proxy = ProxyServer::new(proxy_port);
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

    // Wait for servers to be ready
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        proxy_rx.await.ok();
        control_rx.await.ok();
    }).await;
    tracing::info!("Proxy+control started for '{}': proxy={}, control={}", label, proxy_port, control_port);
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

    // Start proxy + control servers
    start_worker_proxy(proxy_port, control_port, &label).await;

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
    let pane_model_for_config = pane_model.clone();
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
        last_sdk_activity: None,
        scroll_offset: 0,
    });

    // Apply the pane's provider config (from saved config), not app-level
    if let Some(provider) = pane_provider.and_then(|i| app.providers.get(i)) {
        if provider.id != "__default__" {
            let body = serde_json::json!({
                "target_url": provider.base_url,
                "api_format": provider.api_format,
                "api_key": provider.api_key,
                "model": pane_model_for_config,
            });
            let client = reqwest::Client::new();
            match client
                .post(format!("http://127.0.0.1:{}/legion/config", control_port))
                .json(&body)
                .send()
                .await
            {
                Ok(r) => tracing::info!("Worker '{}' proxy configured: {}", label, r.status()),
                Err(e) => tracing::error!("Failed to configure proxy for '{}': {}", label, e),
            }
        }
    }

    // Resize all panes to accommodate the new worker
    app.apply_resize();

    // Persist updated worker count to DB
    app.persist_worker_count();

    tracing::info!("Worker '{}' added successfully", label);
}

/// Send proxy configs to all worker panes synchronously (awaited, not fire-and-forget).
/// Used after start_session to ensure proxy config is in place before SDK starts.
async fn send_proxy_configs_sync(app: &App) {
    let client = reqwest::Client::new();
    for pane in &app.panes {
        let provider = match pane.current_provider.and_then(|i| app.providers.get(i)) {
            Some(p) if p.id != "__default__" => p,
            _ => continue,
        };
        let body = serde_json::json!({
            "target_url": provider.base_url,
            "api_format": provider.api_format,
            "api_key": provider.api_key,
            "model": pane.current_model,
        });
        match client
            .post(format!("http://127.0.0.1:{}/legion/config", pane.control_port))
            .json(&body)
            .send()
            .await
        {
            Ok(r) => tracing::info!("Worker '{}' proxy configured: {}", pane.label, r.status()),
            Err(e) => tracing::error!("Failed to configure proxy for '{}': {}", pane.label, e),
        }
    }
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
            // Output style (plan/normal/etc.)
            if let Some(style_name) = json.get("output_style")
                .and_then(|os| os.get("name"))
                .and_then(|v| v.as_str())
            {
                app.leader_output_style = Some(style_name.to_string());
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
