# SDK Kanban + Async Queue + Ralph Loop Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace Worker PTY execution with SDK stream-json, add shared async task queue with Ralph Loop iteration, configurable Team modes, and embedded Task Board UI.

**Architecture:** Workers execute via `claude -p --output-format=stream-json` (SDK mode). A shared task queue holds all tickets; workers pull one-by-one. Each execution runs inside a custom Ralph Loop that checks for `<promise>DONE</promise>` and retries with feedback. Team composition (TL+Engineer+QA, Solo, Custom) is configurable per ticket.

**Tech Stack:** Rust, tokio, ratatui, tui-term, vt100, serde_json, hyper

---

### Task 1: Refactor OrchestrateEngine — Shared Task Queue

**Files:**
- Modify: `crates/legion-core/src/orchestrate/engine.rs`
- Modify: `crates/legion-core/src/orchestrate/mod.rs`
- Modify: `crates/legion-core/src/lib.rs`

**Step 1: Add new data types**

Add these types above the existing `OrchestrateEngine` struct in `engine.rs`:

```rust
/// Status of a ticket in the shared queue
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketStatus {
    Queued,
    Working,
    Done,
    Error,
}

/// Team execution mode for a ticket
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamMode {
    TechLeadTeam,
    Solo,
    Custom(String),
}

impl Default for TeamMode {
    fn default() -> Self { Self::TechLeadTeam }
}

/// A ticket in the shared task queue
#[derive(Debug, Clone, Serialize)]
pub struct TaskTicket {
    pub id: usize,
    pub prompt: String,
    pub status: TicketStatus,
    pub assigned_worker: Option<u16>,
    pub team_mode: TeamMode,
    pub iteration: u16,
    pub max_iterations: u16,
    pub feedback: Option<String>,
    pub summary: Option<String>,
    pub started_at: Option<std::time::Instant>,
}

impl TaskTicket {
    pub fn elapsed_secs(&self) -> u64 {
        self.started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0)
    }
}

/// Snapshot of a ticket for API/UI consumption (no Instant field)
#[derive(Debug, Clone, Serialize)]
pub struct TicketSnapshot {
    pub id: usize,
    pub prompt: String,
    pub status: TicketStatus,
    pub assigned_worker: Option<u16>,
    pub team_mode: TeamMode,
    pub iteration: u16,
    pub max_iterations: u16,
    pub feedback: Option<String>,
    pub summary: Option<String>,
    pub elapsed_secs: u64,
}
```

**Step 2: Refactor OrchestrateEngine internals**

Replace the `inner: Arc<RwLock<HashMap<u16, WorkerInner>>>` with a new inner struct:

```rust
struct EngineInner {
    tickets: Vec<TaskTicket>,
    next_ticket_id: usize,
    worker_count: u16,
}

#[derive(Clone)]
pub struct OrchestrateEngine {
    inner: Arc<RwLock<EngineInner>>,
}
```

**Step 3: Implement new engine methods**

Replace all existing methods on `OrchestrateEngine` with:

```rust
impl OrchestrateEngine {
    pub fn new(worker_count: u16) -> Self {
        Self {
            inner: Arc::new(RwLock::new(EngineInner {
                tickets: Vec::new(),
                next_ticket_id: 1,
                worker_count,
            })),
        }
    }

    /// Leader submits a new ticket to the queue.
    pub async fn submit_ticket(&self, prompt: String, team_mode: TeamMode, max_iterations: u16) -> usize {
        let mut guard = self.inner.write().await;
        let id = guard.next_ticket_id;
        guard.next_ticket_id += 1;
        guard.tickets.push(TaskTicket {
            id,
            prompt,
            status: TicketStatus::Queued,
            assigned_worker: None,
            team_mode,
            iteration: 0,
            max_iterations,
            feedback: None,
            summary: None,
            started_at: None,
        });
        id
    }

    /// Worker takes the next queued ticket. Returns ticket id and prompt if available.
    pub async fn take_next(&self, worker_id: u16) -> Option<(usize, String, TeamMode)> {
        let mut guard = self.inner.write().await;
        // Check worker isn't already working on something
        let already_working = guard.tickets.iter().any(|t| {
            t.assigned_worker == Some(worker_id) && t.status == TicketStatus::Working
        });
        if already_working { return None; }

        let ticket = guard.tickets.iter_mut().find(|t| t.status == TicketStatus::Queued)?;
        ticket.status = TicketStatus::Working;
        ticket.assigned_worker = Some(worker_id);
        ticket.iteration = 1;
        ticket.started_at = Some(std::time::Instant::now());
        Some((ticket.id, ticket.prompt.clone(), ticket.team_mode.clone()))
    }

    /// Report iteration result for a ticket. Returns true if ticket should retry.
    pub async fn report_iteration(
        &self, ticket_id: usize, success: bool, feedback: Option<String>,
    ) -> bool {
        let mut guard = self.inner.write().await;
        let ticket = match guard.tickets.iter_mut().find(|t| t.id == ticket_id) {
            Some(t) => t,
            None => return false,
        };

        if success {
            ticket.status = TicketStatus::Done;
            ticket.summary = feedback;
            return false; // no retry
        }

        // Failed — check if we can retry
        ticket.iteration += 1;
        if ticket.iteration > ticket.max_iterations {
            ticket.status = TicketStatus::Error;
            ticket.summary = feedback;
            return false; // max retries exceeded
        }

        // Will retry — store feedback, keep status Working
        ticket.feedback = feedback;
        true // should retry
    }

    /// Get the current ticket a worker is working on.
    pub async fn worker_ticket(&self, worker_id: u16) -> Option<TicketSnapshot> {
        let guard = self.inner.read().await;
        guard.tickets.iter()
            .find(|t| t.assigned_worker == Some(worker_id) && t.status == TicketStatus::Working)
            .map(ticket_to_snapshot)
    }

    /// Check if a worker is idle (not working on any ticket).
    pub async fn is_worker_idle(&self, worker_id: u16) -> bool {
        let guard = self.inner.read().await;
        !guard.tickets.iter().any(|t| {
            t.assigned_worker == Some(worker_id) && t.status == TicketStatus::Working
        })
    }

    /// Return all tickets as snapshots.
    pub async fn all_tickets(&self) -> Vec<TicketSnapshot> {
        let guard = self.inner.read().await;
        guard.tickets.iter().map(ticket_to_snapshot).collect()
    }

    /// Queue stats: (total, queued, working, done, error)
    pub async fn queue_stats(&self) -> (usize, usize, usize, usize, usize) {
        let guard = self.inner.read().await;
        let total = guard.tickets.len();
        let queued = guard.tickets.iter().filter(|t| t.status == TicketStatus::Queued).count();
        let working = guard.tickets.iter().filter(|t| t.status == TicketStatus::Working).count();
        let done = guard.tickets.iter().filter(|t| t.status == TicketStatus::Done).count();
        let error = guard.tickets.iter().filter(|t| t.status == TicketStatus::Error).count();
        (total, queued, working, done, error)
    }

    pub async fn worker_count(&self) -> u16 {
        self.inner.read().await.worker_count
    }

    pub async fn set_worker_count(&self, count: u16) {
        self.inner.write().await.worker_count = count;
    }

    /// Stop all working tickets (set to Error).
    pub async fn stop_all(&self) {
        let mut guard = self.inner.write().await;
        for ticket in guard.tickets.iter_mut() {
            if ticket.status == TicketStatus::Working {
                ticket.status = TicketStatus::Error;
                ticket.summary = Some("Stopped by user".into());
            }
        }
    }
}

fn ticket_to_snapshot(t: &TaskTicket) -> TicketSnapshot {
    TicketSnapshot {
        id: t.id,
        prompt: t.prompt.clone(),
        status: t.status,
        assigned_worker: t.assigned_worker,
        team_mode: t.team_mode.clone(),
        iteration: t.iteration,
        max_iterations: t.max_iterations,
        feedback: t.feedback.clone(),
        summary: t.summary.clone(),
        elapsed_secs: t.elapsed_secs(),
    }
}
```

**Step 4: Update mod.rs exports**

In `crates/legion-core/src/orchestrate/mod.rs`, update:

```rust
pub use engine::{
    OrchestrateEngine, TaskTicket, TeamMode, TicketSnapshot, TicketStatus,
};
```

Remove `WorkerState` and `WorkerTaskStatus` from exports.

**Step 5: Update lib.rs re-exports**

In `crates/legion-core/src/lib.rs`, change:

```rust
pub use orchestrate::{OrchestrateApi, OrchestrateEngine, TicketSnapshot, TicketStatus, TeamMode};
```

Remove old `WorkerState` and `WorkerTaskStatus` re-exports.

**Step 6: Compile check**

Run: `cargo check -p legion-core 2>&1 | head -30`

Expected: Errors in `api.rs` (references old types) and downstream crates. We fix api.rs next step.

**Step 7: Commit**

```bash
git add crates/legion-core/src/orchestrate/engine.rs crates/legion-core/src/orchestrate/mod.rs crates/legion-core/src/lib.rs
git commit -m "refactor(orchestrate): shared task queue replacing per-worker state"
```

---

### Task 2: Update Orchestrate API for Task Queue

**Files:**
- Modify: `crates/legion-core/src/orchestrate/api.rs`

**Step 1: Replace dispatch handler with submit_ticket**

Replace `handle_dispatch` with `handle_submit`:

```rust
/// POST /legion/orchestrate/submit — Leader submits a new ticket to the queue.
async fn handle_submit(
    req: Request<Incoming>,
    engine: OrchestrateEngine,
) -> Result<Response<Full<Bytes>>, Infallible> {
    use http_body_util::BodyExt;

    let body_bytes = match req.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            error!("Failed to read submit body: {}", e);
            return Ok(json_response(StatusCode::BAD_REQUEST, r#"{"error": "failed to read body"}"#));
        }
    };

    #[derive(serde::Deserialize)]
    struct SubmitRequest {
        ticket: String,
        #[serde(default)]
        team_mode: Option<super::engine::TeamMode>,
        #[serde(default = "default_max_iter")]
        max_iterations: u16,
    }
    fn default_max_iter() -> u16 { 5 }

    match serde_json::from_slice::<SubmitRequest>(&body_bytes) {
        Ok(req) => {
            let mode = req.team_mode.unwrap_or_default();
            let id = engine.submit_ticket(req.ticket, mode, req.max_iterations).await;
            Ok(json_response(StatusCode::OK, &format!(r#"{{"ticket_id": {}}}"#, id)))
        }
        Err(e) => {
            error!("Invalid submit JSON: {}", e);
            Ok(json_response(StatusCode::BAD_REQUEST, &format!(r#"{{"error": "{}"}}"#, e)))
        }
    }
}
```

**Step 2: Update handle_status to return tickets**

```rust
async fn handle_status(engine: OrchestrateEngine) -> Result<Response<Full<Bytes>>, Infallible> {
    let tickets = engine.all_tickets().await;
    let (total, queued, working, done, error) = engine.queue_stats().await;

    #[derive(serde::Serialize)]
    struct StatusResponse {
        tickets: Vec<super::engine::TicketSnapshot>,
        total: usize,
        queued: usize,
        working: usize,
        done: usize,
        error: usize,
    }

    let resp = StatusResponse { tickets, total, queued, working, done, error };
    match serde_json::to_string(&resp) {
        Ok(json) => Ok(json_response(StatusCode::OK, &json)),
        Err(e) => {
            error!("Failed to serialize status: {}", e);
            Ok(json_response(StatusCode::INTERNAL_SERVER_ERROR, r#"{"error": "serialization failed"}"#))
        }
    }
}
```

**Step 3: Update handle_report for iteration-based reporting**

```rust
async fn handle_report(
    req: Request<Incoming>,
    engine: OrchestrateEngine,
) -> Result<Response<Full<Bytes>>, Infallible> {
    use http_body_util::BodyExt;

    let body_bytes = match req.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            error!("Failed to read report body: {}", e);
            return Ok(json_response(StatusCode::BAD_REQUEST, r#"{"error": "failed to read body"}"#));
        }
    };

    #[derive(serde::Deserialize)]
    struct ReportRequest {
        worker_id: u16,
        status: String,
        summary: Option<String>,
    }

    match serde_json::from_slice::<ReportRequest>(&body_bytes) {
        Ok(req) => {
            let success = req.status == "done";
            // Find the ticket this worker is working on
            if let Some(ticket) = engine.worker_ticket(req.worker_id).await {
                let _should_retry = engine.report_iteration(ticket.id, success, req.summary).await;
                Ok(json_response(StatusCode::OK, r#"{"status": "ok"}"#))
            } else {
                Ok(json_response(StatusCode::BAD_REQUEST, r#"{"error": "worker has no active ticket"}"#))
            }
        }
        Err(e) => {
            error!("Invalid report JSON: {}", e);
            Ok(json_response(StatusCode::BAD_REQUEST, &format!(r#"{{"error": "{}"}}"#, e)))
        }
    }
}
```

**Step 4: Update route table**

In `handle_request`, change dispatch route to submit:

```rust
(Method::POST, "/legion/orchestrate/submit") => handle_submit(req, engine).await,
```

Keep `/legion/orchestrate/dispatch` as an alias pointing to a compat handler that wraps submit (so existing `legion-dispatch` tool still works):

```rust
(Method::POST, "/legion/orchestrate/dispatch") => handle_dispatch_compat(req, engine).await,
```

```rust
/// Backward compat: dispatch assigns ticket to queue (ignores worker_id).
async fn handle_dispatch_compat(
    req: Request<Incoming>,
    engine: OrchestrateEngine,
) -> Result<Response<Full<Bytes>>, Infallible> {
    use http_body_util::BodyExt;
    let body_bytes = match req.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => return Ok(json_response(StatusCode::BAD_REQUEST, r#"{"error": "bad body"}"#)),
    };
    #[derive(serde::Deserialize)]
    struct DispatchRequest { ticket: String, #[serde(default)] worker_id: Option<u16> }
    match serde_json::from_slice::<DispatchRequest>(&body_bytes) {
        Ok(req) => {
            let id = engine.submit_ticket(req.ticket, TeamMode::default(), 5).await;
            Ok(json_response(StatusCode::OK, &format!(r#"{{"ticket_id": {}, "status": "dispatched"}}"#, id)))
        }
        Err(e) => Ok(json_response(StatusCode::BAD_REQUEST, &format!(r#"{{"error": "{}"}}"#, e))),
    }
}
```

Add `use super::engine::TeamMode;` at the top of api.rs.

**Step 5: Compile check**

Run: `cargo check -p legion-core 2>&1 | head -30`

Expected: legion-core compiles. Downstream crates (legion-tui) will have errors — fixed in later tasks.

**Step 6: Commit**

```bash
git add crates/legion-core/src/orchestrate/api.rs
git commit -m "feat(orchestrate): update API for task queue (submit, ticket-based status)"
```

---

### Task 3: Add Promise Detection to sdk.rs

**Files:**
- Modify: `crates/legion-tui/src/sdk.rs`

**Step 1: Add promise detection function**

Add at the bottom of `sdk.rs` (before tests if any):

```rust
/// Check if SDK Result text contains the completion promise tag.
/// Returns Some(true) if <promise>DONE</promise> found, Some(false) if Result
/// exists but no promise, None if no Result yet.
pub fn detect_promise(result_text: &str) -> bool {
    result_text.contains("<promise>DONE</promise>")
        || result_text.contains("<promise>COMPLETE</promise>")
}

/// Extract feedback from a non-promise Result text for Ralph Loop retry.
pub fn extract_feedback(result_text: &str) -> String {
    // Remove promise tags if partially present, trim
    let cleaned = result_text
        .replace("<promise>", "")
        .replace("</promise>", "");
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "Task did not complete. No specific feedback provided.".to_string()
    } else {
        truncate(trimmed, 500)
    }
}
```

**Step 2: Make SdkHandle store the Result text**

Add a field to `SdkHandle`:

```rust
pub struct SdkHandle {
    child: Child,
    pub entry_rx: mpsc::UnboundedReceiver<ProgressEntry>,
    pub finished: Arc<Mutex<bool>>,
    pub success: Arc<Mutex<Option<bool>>>,
    pub result_text: Arc<Mutex<Option<String>>>,  // NEW
}
```

In `SdkHandle::spawn`, initialize it:

```rust
let result_text = Arc::new(Mutex::new(None));
```

In the stdout reader task, when processing `ClaudeJson::Result`, store the result:

```rust
ClaudeJson::Result { result, is_error, .. } => {
    // Store result text for promise detection
    if let Ok(mut rt) = result_text_clone.lock() {
        *rt = result.clone();
    }
    // ... existing code ...
}
```

Clone `result_text` for the reader task and include in `Ok(Self { ... })`.

**Step 3: Add convenience method to get result text**

```rust
/// Get the result text (for promise detection after completion)
pub fn result_text(&self) -> Option<String> {
    self.result_text.lock().ok().and_then(|r| r.clone())
}
```

**Step 4: Update spawn to accept iteration and feedback params**

Add `iteration` and `feedback` parameters to build the full prompt:

```rust
pub fn spawn(
    working_dir: &Path,
    prompt: &str,
    parser: SharedParser,
    use_proxy: bool,
    proxy_port: u16,
    system_prompt: Option<&str>,
    iteration: u16,        // NEW
    feedback: Option<&str>, // NEW
) -> anyhow::Result<Self> {
    // Build the effective prompt
    let effective_prompt = if iteration > 1 {
        if let Some(fb) = feedback {
            format!(
                "{}\n\n--- Previous Iteration Feedback (attempt {}) ---\n{}\n--- End Feedback ---\n\nPlease address the feedback above and complete the task. Output <promise>DONE</promise> when finished.",
                prompt, iteration - 1, fb
            )
        } else {
            prompt.to_string()
        }
    } else {
        prompt.to_string()
    };

    let mut cmd = Command::new("claude");
    cmd.arg("-p")
        .arg(&effective_prompt)  // use effective_prompt
        // ... rest unchanged ...
```

**Step 5: Compile check**

Run: `cargo check -p legion-tui 2>&1 | head -30`

Expected: Errors in files that call `SdkHandle::spawn` with old signature. Fixed in later tasks.

**Step 6: Commit**

```bash
git add crates/legion-tui/src/sdk.rs
git commit -m "feat(sdk): add promise detection, feedback injection, result text storage"
```

---

### Task 4: Refactor App State for SDK Workers

**Files:**
- Modify: `crates/legion-tui/src/app.rs`

**Step 1: Add SDK fields to Pane**

Add imports at top:

```rust
use crate::sdk::{SdkHandle, ProgressEntry};
```

Add fields to `Pane` struct:

```rust
pub struct Pane {
    pub pty: Option<PtyHandle>,
    pub proxy_port: u16,
    pub control_port: u16,
    pub label: String,
    pub current_provider: Option<usize>,
    pub current_model: Option<String>,
    pub spawned_with_continue: bool,
    // SDK execution state (workers only)
    pub sdk_task: Option<SdkHandle>,
    pub sdk_parser: Option<SharedParser>,
    pub sdk_entries: Vec<ProgressEntry>,
    pub current_ticket_id: Option<usize>,
}
```

Update Pane's `parser()` to return sdk_parser for workers:

```rust
impl Pane {
    pub fn parser(&self) -> Option<&SharedParser> {
        // Prefer SDK parser (workers), fallback to PTY parser (leader)
        self.sdk_parser.as_ref().or_else(|| self.pty.as_ref().map(|pty| &pty.parser))
    }
}
```

**Step 2: Update App struct**

Remove old fields, add new ones:

```rust
pub struct App {
    // ... keep all existing fields except:
    // REMOVE: pub show_dashboard: bool,
    // ADD:
    pub right_panel_focused: bool,
    pub ticket_snapshot: Option<Vec<legion_core::TicketSnapshot>>,
    pub queue_stats: Option<(usize, usize, usize, usize, usize)>,
    // KEEP: kanban_selected, kanban_detail, kanban_detail_scroll
    // ...
}
```

Update `App::new()`: remove `show_dashboard: false`, add `right_panel_focused: false`, `ticket_snapshot: None`, `queue_stats: None`. Initialize new Pane fields in `add_pane`:

```rust
self.panes.push(Pane {
    pty,
    proxy_port,
    control_port,
    label,
    current_provider: pane_provider,
    current_model: pane_model,
    spawned_with_continue: continue_session,
    sdk_task: None,
    sdk_parser: None,
    sdk_entries: Vec::new(),
    current_ticket_id: None,
});
```

**Step 3: Remove capture_pane_screen**

Delete the `capture_pane_screen` method entirely (lines ~736-745).

**Step 4: Update kill_all to also kill SDK tasks**

```rust
pub fn kill_all(&mut self) {
    for pane in &mut self.panes {
        if let Some(ref mut pty) = pane.pty {
            pty.kill();
        }
        if let Some(ref mut sdk) = pane.sdk_task {
            sdk.kill();
        }
    }
}
```

**Step 5: Update remove_single_worker to handle SDK**

In `remove_single_worker`, after "Kill PTY" section, add SDK kill:

```rust
if let Some(ref mut sdk) = self.panes[pane_index].sdk_task {
    sdk.kill();
}
```

**Step 6: Add start_sdk_task method**

```rust
/// Start an SDK task on a worker pane
pub fn start_sdk_task(
    &mut self,
    pane_index: usize,
    ticket_id: usize,
    prompt: &str,
    team_mode: &legion_core::TeamMode,
    iteration: u16,
    feedback: Option<&str>,
) {
    // Compute values before mutable borrow
    let working_dir = self.pane_worktree(&self.panes[pane_index].label.clone());
    let use_proxy = self.pane_uses_proxy(&self.panes[pane_index].label.clone());
    let proxy_port = self.panes[pane_index].proxy_port;

    // Create SDK parser
    let (term_w, term_h) = self.term_size;
    let lw = (term_w as u32 * self.leader_ratio as u32 / 100) as u16;
    let ww = term_w.saturating_sub(lw).saturating_sub(1);
    let parser = std::sync::Arc::new(std::sync::Mutex::new(
        vt100::Parser::new(term_h.saturating_sub(4), ww.saturating_sub(2), 1000)
    ));

    // Generate system prompt based on team mode
    let sys_prompt = match team_mode {
        legion_core::TeamMode::TechLeadTeam => {
            let worker_id = pane_index as u16;
            crate::claudemd::worker_instructions(worker_id)
        }
        legion_core::TeamMode::Solo => {
            "You are a solo developer. Follow TDD: write failing tests first, then implement. Run tests to verify. Output <promise>DONE</promise> when complete.".to_string()
        }
        legion_core::TeamMode::Custom(desc) => {
            format!("{}\n\nOutput <promise>DONE</promise> when complete.", desc)
        }
    };

    let wd = working_dir.unwrap_or_else(|| std::path::PathBuf::from("."));

    match crate::sdk::SdkHandle::spawn(
        &wd, prompt, parser.clone(), use_proxy, proxy_port,
        Some(&sys_prompt), iteration, feedback,
    ) {
        Ok(handle) => {
            let pane = &mut self.panes[pane_index];
            pane.sdk_task = Some(handle);
            pane.sdk_parser = Some(parser);
            pane.current_ticket_id = Some(ticket_id);
            if iteration == 1 {
                pane.sdk_entries.clear();
            }
            tracing::info!("SDK task started for pane {} (ticket {}, iter {})", pane_index, ticket_id, iteration);
        }
        Err(e) => {
            tracing::error!("Failed to start SDK task for pane {}: {}", pane_index, e);
        }
    }
}
```

**Step 7: Compile check**

Run: `cargo check -p legion-tui 2>&1 | head -40`

Expected: Errors in `ui.rs` (references `show_dashboard`, `WorkerTaskStatus`), `input.rs` (Ctrl+T), `lib.rs` (old dispatch). Fixed in later tasks.

**Step 8: Commit**

```bash
git add crates/legion-tui/src/app.rs
git commit -m "refactor(app): add SDK state to Pane, remove show_dashboard, add start_sdk_task"
```

---

### Task 5: Update Event Loop — SDK Dispatch + Ralph Loop

**Files:**
- Modify: `crates/legion-tui/src/lib.rs`

**Step 1: Remove PTY injection block**

Delete the entire block in `run_event_loop` that does PTY injection (lines ~202-234):

```rust
// DELETE: Poll for pending tasks and inject into idle Worker PTYs
// ... the entire if let Some(engine) block with injections ...
```

**Step 2: Add SDK dispatch + Ralph Loop logic**

Replace the deleted block with:

```rust
// --- SDK dispatch: idle workers pull from queue ---
if let Some(engine) = app.orchestrate.clone() {
    // Update ticket snapshots for UI
    app.ticket_snapshot = Some(engine.all_tickets().await);
    app.queue_stats = Some(engine.queue_stats().await);

    let wc = engine.worker_count().await;

    // For each worker pane, check if SDK is done or if we should start new task
    for wi in 1..=wc as usize {
        if wi >= app.panes.len() { break; }

        let pane = &mut app.panes[wi];

        // Drain new SDK entries
        if let Some(ref mut sdk) = pane.sdk_task {
            pane.sdk_entries.extend(sdk.drain_entries());

            // Check if SDK finished
            if sdk.is_finished() {
                let result_text = sdk.result_text().unwrap_or_default();
                let promise_found = crate::sdk::detect_promise(&result_text);
                let ticket_id = pane.current_ticket_id.unwrap_or(0);

                if promise_found {
                    // Success — report to engine
                    let summary = Some(crate::sdk::extract_feedback(&result_text));
                    engine.report_iteration(ticket_id, true, summary).await;
                    tracing::info!("Worker {} ticket {} completed (promise found)", wi, ticket_id);
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
                            // Clean up old SDK
                            pane.sdk_task = None;
                            // Start new iteration
                            app.start_sdk_task(wi, ticket_id, &prompt, &team_mode, ts.iteration, Some(&feedback));
                            continue;
                        }
                    } else {
                        tracing::warn!("Worker {} ticket {} failed (max iterations)", wi, ticket_id);
                    }
                }

                // Clean up finished SDK
                app.panes[wi].sdk_task = None;
                app.panes[wi].current_ticket_id = None;
            }
        }

        // If worker is idle and no SDK running, try to take next ticket
        if app.panes[wi].sdk_task.is_none() {
            if let Some((ticket_id, prompt, team_mode)) = engine.take_next(wi as u16).await {
                tracing::info!("Worker {} taking ticket {}", wi, ticket_id);
                app.start_sdk_task(wi, ticket_id, &prompt, &team_mode, 1, None);
            }
        }
    }
}
```

**Step 3: Update orchestrate snapshot references**

Remove the old orchestrate_snapshot update block (lines ~178-180):

```rust
// DELETE:
// if let Some(engine) = app.orchestrate.clone() {
//     app.orchestrate_snapshot = Some(engine.all_status().await);
// }
```

This is now done inside the new SDK dispatch block.

**Step 4: Remove Worker PTY spawn from start_session**

In `App::start_session()` (in `app.rs`), workers should NOT spawn PTY. Change the worker spawn loop to create panes without PTY:

```rust
// Spawn worker panes (NO PTY — workers use SDK)
for i in 0..worker_count {
    let proxy = base_port + i + 1;
    let control = base_port + 1000 + i + 1;
    let label = format!("Worker {}", i + 1);
    // Add pane without PTY (sdk_task will be set when ticket is assigned)
    self.panes.push(Pane {
        pty: None,
        proxy_port: proxy,
        control_port: control,
        label,
        current_provider: self.current_provider,
        current_model: self.current_model.clone(),
        spawned_with_continue: false,
        sdk_task: None,
        sdk_parser: None,
        sdk_entries: Vec::new(),
        current_ticket_id: None,
    });
}
```

Remove the worker's `add_pane()` call and the worker PTY size calculations that are no longer needed.

**Step 5: Update handle_add_worker in lib.rs**

Similarly, `handle_add_worker` should create worker pane without PTY:

```rust
// Add pane WITHOUT PTY (SDK will be used when ticket assigned)
app.panes.push(crate::app::Pane {
    pty: None,
    proxy_port,
    control_port,
    label: label.clone(),
    current_provider: app.current_provider,
    current_model: app.current_model.clone(),
    spawned_with_continue: false,
    sdk_task: None,
    sdk_parser: None,
    sdk_entries: Vec::new(),
    current_ticket_id: None,
});
```

Remove the PTY size calculations and `add_pane` call. Keep the proxy/control server setup and worktree creation.

**Step 6: Remove old imports**

Remove `use pty::is_pty_idle;` and `use legion_core::WorkerTaskStatus;` from `lib.rs` if present.

**Step 7: Compile check**

Run: `cargo check -p legion-tui 2>&1 | head -40`

**Step 8: Commit**

```bash
git add crates/legion-tui/src/lib.rs crates/legion-tui/src/app.rs
git commit -m "feat(lib): SDK dispatch with Ralph Loop, remove PTY injection"
```

---

### Task 6: Update UI — Embedded Task Board

**Files:**
- Modify: `crates/legion-tui/src/ui.rs`

**Step 1: Update imports**

Replace old imports:

```rust
use legion_core::orchestrate::WorkerTaskStatus;
```

with:

```rust
use legion_core::{TicketSnapshot, TicketStatus};
```

**Step 2: Remove old dashboard overlay**

Delete these functions entirely:
- `draw_dashboard_overlay`
- `draw_kanban_board` (the popup version)
- `draw_kanban_detail` (the popup version)

Remove `show_dashboard` check from `draw()`:

```rust
// DELETE:
// if app.show_dashboard {
//     draw_dashboard_overlay(frame, app);
// }
```

**Step 3: Rewrite draw_squad_layout**

Right side is now Task Board, not worker PTY panes:

```rust
fn draw_squad_layout(frame: &mut Frame, app: &App, area: Rect) {
    let leader_width = (area.width as u32 * app.leader_ratio as u32 / 100) as u16;
    let h_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(leader_width),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(area);

    // Left: Leader PTY
    draw_pane(frame, app, 0, h_chunks[0]);

    // Center: divider
    draw_divider(frame, app, h_chunks[1]);

    // Right: Task Board (kanban)
    if app.kanban_detail {
        draw_ticket_detail(frame, app, h_chunks[2]);
    } else {
        draw_task_board(frame, app, h_chunks[2]);
    }
}
```

**Step 4: Implement draw_task_board (embedded version)**

```rust
fn draw_task_board(frame: &mut Frame, app: &App, area: Rect) {
    let is_focused = app.right_panel_focused;
    let border_color = if is_focused { Color::Blue } else { Color::DarkGray };

    let block = Block::default()
        .title(" Task Board ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let tickets = app.ticket_snapshot.as_deref().unwrap_or(&[]);
    if tickets.is_empty() {
        let content = Paragraph::new(" Waiting for Leader to submit tickets...")
            .style(Style::default().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(content, area);
        return;
    }

    let mut items: Vec<ListItem> = Vec::new();

    // Working tickets first
    let working: Vec<&TicketSnapshot> = tickets.iter().filter(|t| t.status == TicketStatus::Working).collect();
    if !working.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            " WORKING", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ))));
        for t in &working {
            items.push(ticket_list_item(t, app));
        }
    }

    // Queued tickets
    let queued: Vec<&TicketSnapshot> = tickets.iter().filter(|t| t.status == TicketStatus::Queued).collect();
    if !queued.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            " QUEUED", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ))));
        for t in &queued {
            items.push(ticket_list_item(t, app));
        }
    }

    // Done tickets
    let done: Vec<&TicketSnapshot> = tickets.iter().filter(|t| t.status == TicketStatus::Done).collect();
    if !done.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            " DONE", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
        ))));
        for t in &done {
            items.push(ticket_list_item(t, app));
        }
    }

    // Error tickets
    let errored: Vec<&TicketSnapshot> = tickets.iter().filter(|t| t.status == TicketStatus::Error).collect();
    if !errored.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            " ERROR", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        ))));
        for t in &errored {
            items.push(ticket_list_item(t, app));
        }
    }

    frame.render_widget(List::new(items).block(block), area);
}

fn ticket_list_item<'a>(ticket: &TicketSnapshot, app: &App) -> ListItem<'a> {
    let selected = app.kanban_selected == ticket.id;
    let prefix = if selected && app.right_panel_focused { "\u{25b6} " } else { "  " };

    let prompt_short = if ticket.prompt.len() > 30 {
        format!("{}...", &ticket.prompt[..27])
    } else {
        ticket.prompt.clone()
    };

    let (status_str, status_color) = match ticket.status {
        TicketStatus::Working => {
            let w = ticket.assigned_worker.map(|w| format!("W{}", w)).unwrap_or_default();
            (format!("{} [{}/{}]", w, ticket.iteration, ticket.max_iterations), Color::Yellow)
        }
        TicketStatus::Queued => ("".into(), Color::DarkGray),
        TicketStatus::Done => (format_elapsed(ticket.elapsed_secs), Color::Green),
        TicketStatus::Error => (format!("ERR {}/{}",ticket.iteration, ticket.max_iterations), Color::Red),
    };

    let row_style = if selected && app.right_panel_focused {
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    };

    ListItem::new(Line::from(vec![
        Span::styled(prefix.to_string(), Style::default().fg(Color::Yellow)),
        Span::styled(format!("#{:<3}", ticket.id), Style::default().fg(Color::DarkGray)),
        Span::styled(prompt_short, row_style),
        Span::styled(format!(" {}", status_str), Style::default().fg(status_color)),
    ]))
}
```

**Step 5: Implement draw_ticket_detail (embedded version)**

```rust
fn draw_ticket_detail(frame: &mut Frame, app: &App, area: Rect) {
    let is_focused = app.right_panel_focused;
    let border_color = if is_focused { Color::Blue } else { Color::DarkGray };

    // Find the selected ticket
    let ticket = app.ticket_snapshot.as_ref()
        .and_then(|ts| ts.iter().find(|t| t.id == app.kanban_selected));

    let title = match ticket {
        Some(t) => format!(" #{} [Esc: back] ", t.id),
        None => " Ticket Detail [Esc: back] ".to_string(),
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    // Try to render the SDK parser output using PseudoTerminal
    // Find the worker pane that has this ticket
    let pane_idx = ticket.and_then(|t| t.assigned_worker).map(|w| w as usize);

    if let Some(idx) = pane_idx {
        if let Some(parser) = app.parser_at(idx) {
            if let Ok(p) = parser.lock() {
                let pseudo_term = PseudoTerminal::new(p.screen()).block(block);
                frame.render_widget(pseudo_term, area);
                return;
            }
        }
    }

    // Fallback: show ticket info as text
    let mut lines = Vec::new();
    if let Some(t) = ticket {
        lines.push(Line::from(vec![
            Span::styled("  Task: ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(t.prompt.clone(), Style::default().fg(Color::White)),
        ]));
        lines.push(Line::from(vec![
            Span::styled(
                format!("  Status: {:?} | Iter: {}/{} | Elapsed: {}",
                    t.status, t.iteration, t.max_iterations, format_elapsed(t.elapsed_secs)),
                Style::default().fg(Color::DarkGray),
            ),
        ]));
        if let Some(ref fb) = t.feedback {
            lines.push(Line::from(Span::raw("")));
            lines.push(Line::from(Span::styled("  Last Feedback:", Style::default().fg(Color::Yellow))));
            lines.push(Line::from(Span::styled(format!("  {}", fb), Style::default().fg(Color::White))));
        }
    } else {
        lines.push(Line::from(Span::styled("  No ticket selected", Style::default().fg(Color::DarkGray))));
    }

    frame.render_widget(Paragraph::new(lines).block(block), area);
}
```

**Step 6: Update draw_header with queue stats**

Add queue stats after the session name:

```rust
// In draw_header, after session name block, add:
if app.is_squad() {
    if let Some((total, _queued, working, done, error)) = app.queue_stats {
        spans.push(Span::styled("  ", Style::default()));
        spans.push(Span::styled(format!("W:{}", app.worker_count()), Style::default().fg(Color::Cyan)));
        spans.push(Span::styled(format!(" Q:{}", total), Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(format!(" \u{2713}{}", done), Style::default().fg(Color::Green)));
        if working > 0 {
            spans.push(Span::styled(format!(" \u{25b6}{}", working), Style::default().fg(Color::Yellow)));
        }
        if error > 0 {
            spans.push(Span::styled(format!(" \u{2717}{}", error), Style::default().fg(Color::Red)));
        }
    }
}
```

**Step 7: Update draw_pane — remove worker task status references**

Remove `WorkerTaskStatus` references in `draw_pane`. Simplify the title for workers since they no longer have per-pane status display (workers show in Task Board now). The `draw_pane` function should only be called for the Leader pane in squad mode.

**Step 8: Update draw_footer**

Remove `Ctrl+T: Dashboard` from footer. Update squad footer hints:

```rust
// Squad mode footer
vec![
    Span::styled(" Alt+\u{2190}\u{2192}", Style::default().fg(Color::Yellow)),
    Span::styled(": Focus ", Style::default().fg(Color::DarkGray)),
    Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
    Span::styled("Ctrl+P", Style::default().fg(Color::Yellow)),
    Span::styled(": Menu ", Style::default().fg(Color::DarkGray)),
    Span::styled("\u{2502} ", Style::default().fg(Color::DarkGray)),
    Span::styled("Ctrl+Q", Style::default().fg(Color::Yellow)),
    Span::styled(": Quit", Style::default().fg(Color::DarkGray)),
]
```

**Step 9: Compile check**

Run: `cargo check -p legion-tui 2>&1 | head -40`

**Step 10: Commit**

```bash
git add crates/legion-tui/src/ui.rs
git commit -m "feat(ui): embedded Task Board replacing worker PTY panes"
```

---

### Task 7: Update Input Handling

**Files:**
- Modify: `crates/legion-tui/src/input.rs`

**Step 1: Remove Ctrl+T handler**

Delete the entire Ctrl+T block in `handle_normal_mode`:

```rust
// DELETE:
// if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('t') { ... }
```

**Step 2: Remove show_dashboard kanban key capture**

Delete the block:

```rust
// DELETE:
// if app.show_dashboard && app.is_squad() {
//     return handle_kanban_keys(app, key);
// }
```

**Step 3: Update Alt+Left/Right to toggle left/right focus**

Replace the current `focus_next()` / `focus_prev()` calls with panel focus toggle:

```rust
// Alt+Right / Alt+Left toggle focus between Leader and Task Board
KeyCode::Right if key.modifiers.contains(KeyModifiers::ALT) => {
    app.right_panel_focused = !app.right_panel_focused;
    return InputResult::Continue;
}
KeyCode::Left if key.modifiers.contains(KeyModifiers::ALT) => {
    app.right_panel_focused = !app.right_panel_focused;
    return InputResult::Continue;
}
```

**Step 4: Add Task Board navigation in normal mode**

When `right_panel_focused` is true, intercept j/k/Enter/Esc:

```rust
// Task Board navigation when right panel is focused
if app.right_panel_focused && app.is_squad() {
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            navigate_ticket_down(app);
            return InputResult::Continue;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            navigate_ticket_up(app);
            return InputResult::Continue;
        }
        KeyCode::Enter => {
            if !app.kanban_detail {
                app.kanban_detail = true;
            }
            return InputResult::Continue;
        }
        KeyCode::Esc => {
            if app.kanban_detail {
                app.kanban_detail = false;
            } else {
                app.right_panel_focused = false;
            }
            return InputResult::Continue;
        }
        _ => {
            return InputResult::Continue; // Don't forward to PTY when right panel focused
        }
    }
}
```

Add navigation helpers:

```rust
fn navigate_ticket_down(app: &mut App) {
    if let Some(ref tickets) = app.ticket_snapshot {
        if tickets.is_empty() { return; }
        let ids: Vec<usize> = tickets.iter().map(|t| t.id).collect();
        let current_pos = ids.iter().position(|&id| id == app.kanban_selected).unwrap_or(0);
        let next = if current_pos + 1 < ids.len() { current_pos + 1 } else { 0 };
        app.kanban_selected = ids[next];
    }
}

fn navigate_ticket_up(app: &mut App) {
    if let Some(ref tickets) = app.ticket_snapshot {
        if tickets.is_empty() { return; }
        let ids: Vec<usize> = tickets.iter().map(|t| t.id).collect();
        let current_pos = ids.iter().position(|&id| id == app.kanban_selected).unwrap_or(0);
        let prev = if current_pos > 0 { current_pos - 1 } else { ids.len() - 1 };
        app.kanban_selected = ids[prev];
    }
}
```

**Step 5: Update handle_kanban_keys — delete or simplify**

Delete the old `handle_kanban_keys` function entirely (it was for the popup overlay).

**Step 6: Compile check**

Run: `cargo check -p legion-tui 2>&1 | head -40`

**Step 7: Commit**

```bash
git add crates/legion-tui/src/input.rs
git commit -m "feat(input): Task Board navigation, remove Ctrl+T, panel focus toggle"
```

---

### Task 8: Cleanup — Remove Dead Code

**Files:**
- Modify: `crates/legion-tui/src/app.rs`
- Modify: `crates/legion-tui/src/ui.rs`
- Modify: `crates/legion-tui/src/lib.rs`

**Step 1: Remove orchestrate_snapshot from App**

In `app.rs`, remove:
- `pub orchestrate_snapshot: Option<Vec<WorkerState>>` field
- Its initialization in `App::new()`
- The `worker_task_status()` method that used it
- Import of `WorkerState` from legion_core

**Step 2: Remove unused Worker PTY resize logic**

In `resize_panes()` in `app.rs`, the worker resize loop (lines ~375-379) should be removed or simplified since workers don't have PTY:

```rust
// Workers don't have PTY, nothing to resize
// Only resize leader
```

**Step 3: Remove write_to_pane (for workers)**

Keep `write_to_pty` (for leader), but `write_to_pane` is no longer needed for SDK workers. Can keep it but add a note it's leader-only.

**Step 4: Clean up unused imports**

Remove all `WorkerTaskStatus` and `WorkerState` imports across the TUI crate.

**Step 5: Run full build**

Run: `cargo build 2>&1 | tail -20`

Expected: Clean build.

**Step 6: Run tests**

Run: `cargo test 2>&1 | tail -20`

Expected: All tests pass (some existing tests for `is_pty_idle` and `claudemd` should still pass).

**Step 7: Commit**

```bash
git add -A
git commit -m "cleanup: remove dead PTY worker code, old orchestrate snapshot, unused imports"
```

---

### Task 9: Update legion-tools for Queue API

**Files:**
- Modify: `crates/legion-tools/src/bin/legion-dispatch.rs`
- Modify: `crates/legion-tools/src/bin/legion-check.rs`

**Step 1: Update legion-dispatch to use submit endpoint**

The dispatch tool should submit to the queue (worker_id becomes optional context, not assignment):

```rust
// POST to /legion/orchestrate/submit instead of /dispatch
// Body: {"ticket": "...", "team_mode": "tech_lead_team"}
```

Keep the same CLI interface: `legion-dispatch <worker_id> "ticket"` but internally submit to queue.

**Step 2: Update legion-check to show ticket queue**

Parse the new status response format with `tickets` array instead of `workers` array. Display tickets grouped by status.

**Step 3: Compile and test tools**

Run: `cargo build -p legion-tools 2>&1 | tail -10`

**Step 4: Commit**

```bash
git add crates/legion-tools/
git commit -m "feat(tools): update dispatch/check for task queue API"
```

---

### Task 10: Integration Test

**Step 1: Build everything**

Run: `cargo build 2>&1 | tail -20`

Expected: Clean build with no errors.

**Step 2: Run all tests**

Run: `cargo test 2>&1 | tail -30`

Expected: All tests pass.

**Step 3: Manual smoke test**

Run: `cargo run -- squad --workers 2`

Verify:
- Session selection appears
- Leader PTY works normally on left
- Right panel shows "Waiting for Leader to submit tickets..."
- Alt+Left/Right toggles focus (border color changes)
- Ctrl+P menu works
- When Leader dispatches tickets, they appear in Task Board
- Ctrl+Q exits cleanly

**Step 4: Final commit**

```bash
git add -A
git commit -m "feat: SDK Kanban with async queue, Ralph Loop, and Team modes"
```
