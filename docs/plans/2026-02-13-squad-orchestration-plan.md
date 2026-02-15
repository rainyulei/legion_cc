# Squad Orchestration Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Enable Leader to plan, dispatch, and verify tasks executed autonomously by Workers via PTY injection, with a TUI dashboard showing real-time progress.

**Architecture:** Orchestration Engine runs inside the Legion TUI process, exposing an HTTP API on `base_port + 2000`. CLI tools (`legion-dispatch`, `legion-check`, etc.) POST to this API. The engine injects tasks into Worker PTYs when idle (detected via vt100 parser), and Workers report results via files + CLI.

**Tech Stack:** Rust, hyper (HTTP), vt100 (idle detection), serde_json, tokio, ratatui

---

### Task 1: Orchestration Engine — Core State

The orchestration engine tracks Worker states, task queues, and results. This is the foundation that all other components build on.

**Files:**
- Create: `crates/legion-core/src/orchestrate/mod.rs`
- Create: `crates/legion-core/src/orchestrate/engine.rs`
- Modify: `crates/legion-core/src/lib.rs:1-11`
- Test: `crates/legion-core/tests/orchestrate_engine_test.rs`

**Step 1: Write the failing test**

Create `crates/legion-core/tests/orchestrate_engine_test.rs`:

```rust
use legion_core::orchestrate::{OrchestrateEngine, WorkerTaskStatus};

#[tokio::test]
async fn test_engine_init_workers() {
    let engine = OrchestrateEngine::new(3); // 3 workers
    let status = engine.all_status().await;
    assert_eq!(status.len(), 3);
    for w in &status {
        assert_eq!(w.status, WorkerTaskStatus::Idle);
        assert!(w.ticket.is_none());
    }
}

#[tokio::test]
async fn test_dispatch_task() {
    let engine = OrchestrateEngine::new(2);
    let result = engine.dispatch(1, "Implement JWT auth".to_string()).await;
    assert!(result.is_ok());

    let status = engine.worker_status(1).await.unwrap();
    assert_eq!(status.status, WorkerTaskStatus::Pending);
    assert_eq!(status.ticket.as_deref(), Some("Implement JWT auth"));
}

#[tokio::test]
async fn test_dispatch_invalid_worker() {
    let engine = OrchestrateEngine::new(2);
    let result = engine.dispatch(5, "task".to_string()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_report_done() {
    let engine = OrchestrateEngine::new(2);
    engine.dispatch(1, "task".to_string()).await.unwrap();
    engine.mark_working(1).await; // simulate injection happened
    engine.report(1, WorkerTaskStatus::Done, Some("all tests pass".into())).await.unwrap();

    let status = engine.worker_status(1).await.unwrap();
    assert_eq!(status.status, WorkerTaskStatus::Done);
    assert_eq!(status.summary.as_deref(), Some("all tests pass"));
}

#[tokio::test]
async fn test_report_error() {
    let engine = OrchestrateEngine::new(2);
    engine.dispatch(1, "task".to_string()).await.unwrap();
    engine.mark_working(1).await;
    engine.report(1, WorkerTaskStatus::Error, Some("compilation failed".into())).await.unwrap();

    let status = engine.worker_status(1).await.unwrap();
    assert_eq!(status.status, WorkerTaskStatus::Error);
}

#[tokio::test]
async fn test_take_pending_task() {
    let engine = OrchestrateEngine::new(2);
    engine.dispatch(1, "task A".to_string()).await.unwrap();

    // Take pending task for injection
    let task = engine.take_pending(1).await;
    assert_eq!(task.as_deref(), Some("task A"));

    // After taking, status should be Working
    let status = engine.worker_status(1).await.unwrap();
    assert_eq!(status.status, WorkerTaskStatus::Working);

    // No more pending
    let task2 = engine.take_pending(1).await;
    assert!(task2.is_none());
}

#[tokio::test]
async fn test_stop_worker() {
    let engine = OrchestrateEngine::new(2);
    engine.dispatch(1, "task".to_string()).await.unwrap();
    engine.mark_working(1).await;
    engine.stop_worker(1).await;

    let status = engine.worker_status(1).await.unwrap();
    assert_eq!(status.status, WorkerTaskStatus::Stopped);
}

#[tokio::test]
async fn test_stop_all() {
    let engine = OrchestrateEngine::new(3);
    for i in 1..=3 {
        engine.dispatch(i, format!("task {}", i)).await.unwrap();
        engine.mark_working(i).await;
    }
    engine.stop_all().await;

    for i in 1..=3 {
        let s = engine.worker_status(i).await.unwrap();
        assert_eq!(s.status, WorkerTaskStatus::Stopped);
    }
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p legion-core --test orchestrate_engine_test 2>&1 | head -20`
Expected: FAIL (module not found)

**Step 3: Write minimal implementation**

Create `crates/legion-core/src/orchestrate/mod.rs`:
```rust
mod engine;

pub use engine::{OrchestrateEngine, WorkerState, WorkerTaskStatus};
```

Create `crates/legion-core/src/orchestrate/engine.rs`:
```rust
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Result};
use serde::Serialize;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerTaskStatus {
    Idle,
    Pending,
    Working,
    Done,
    Error,
    Stopped,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkerState {
    pub worker_id: u16,
    pub status: WorkerTaskStatus,
    pub ticket: Option<String>,
    pub summary: Option<String>,
    pub started_at: Option<u64>,  // epoch secs
    pub elapsed_secs: u64,
}

struct InnerWorker {
    worker_id: u16,
    status: WorkerTaskStatus,
    ticket: Option<String>,
    summary: Option<String>,
    started_at: Option<Instant>,
}

pub struct OrchestrateEngine {
    workers: Arc<RwLock<Vec<InnerWorker>>>,
}

impl OrchestrateEngine {
    pub fn new(worker_count: u16) -> Self {
        let workers: Vec<InnerWorker> = (1..=worker_count)
            .map(|id| InnerWorker {
                worker_id: id,
                status: WorkerTaskStatus::Idle,
                ticket: None,
                summary: None,
                started_at: None,
            })
            .collect();
        Self {
            workers: Arc::new(RwLock::new(workers)),
        }
    }

    pub async fn dispatch(&self, worker_id: u16, ticket: String) -> Result<()> {
        let mut workers = self.workers.write().await;
        let w = workers
            .iter_mut()
            .find(|w| w.worker_id == worker_id);
        match w {
            Some(w) => {
                w.ticket = Some(ticket);
                w.status = WorkerTaskStatus::Pending;
                w.summary = None;
                w.started_at = None;
                Ok(())
            }
            None => bail!("worker {} not found", worker_id),
        }
    }

    pub async fn mark_working(&self, worker_id: u16) {
        let mut workers = self.workers.write().await;
        if let Some(w) = workers.iter_mut().find(|w| w.worker_id == worker_id) {
            w.status = WorkerTaskStatus::Working;
            w.started_at = Some(Instant::now());
        }
    }

    pub async fn report(&self, worker_id: u16, status: WorkerTaskStatus, summary: Option<String>) -> Result<()> {
        let mut workers = self.workers.write().await;
        let w = workers
            .iter_mut()
            .find(|w| w.worker_id == worker_id);
        match w {
            Some(w) => {
                w.status = status;
                w.summary = summary;
                Ok(())
            }
            None => bail!("worker {} not found", worker_id),
        }
    }

    pub async fn take_pending(&self, worker_id: u16) -> Option<String> {
        let mut workers = self.workers.write().await;
        let w = workers.iter_mut().find(|w| w.worker_id == worker_id)?;
        if w.status == WorkerTaskStatus::Pending {
            w.status = WorkerTaskStatus::Working;
            w.started_at = Some(Instant::now());
            w.ticket.clone()
        } else {
            None
        }
    }

    pub async fn stop_worker(&self, worker_id: u16) {
        let mut workers = self.workers.write().await;
        if let Some(w) = workers.iter_mut().find(|w| w.worker_id == worker_id) {
            w.status = WorkerTaskStatus::Stopped;
        }
    }

    pub async fn stop_all(&self) {
        let mut workers = self.workers.write().await;
        for w in workers.iter_mut() {
            if matches!(w.status, WorkerTaskStatus::Working | WorkerTaskStatus::Pending) {
                w.status = WorkerTaskStatus::Stopped;
            }
        }
    }

    pub async fn worker_status(&self, worker_id: u16) -> Option<WorkerState> {
        let workers = self.workers.read().await;
        workers.iter().find(|w| w.worker_id == worker_id).map(to_state)
    }

    pub async fn all_status(&self) -> Vec<WorkerState> {
        let workers = self.workers.read().await;
        workers.iter().map(to_state).collect()
    }
}

fn to_state(w: &InnerWorker) -> WorkerState {
    WorkerState {
        worker_id: w.worker_id,
        status: w.status,
        ticket: w.ticket.clone(),
        summary: w.summary.clone(),
        started_at: None,
        elapsed_secs: w.started_at.map(|s| s.elapsed().as_secs()).unwrap_or(0),
    }
}
```

Update `crates/legion-core/src/lib.rs` — add `pub mod orchestrate;` and re-exports:
```rust
pub mod orchestrate;
// ... existing modules ...
pub use orchestrate::{OrchestrateEngine, WorkerState, WorkerTaskStatus};
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p legion-core --test orchestrate_engine_test -- --nocapture`
Expected: all 8 tests PASS

**Step 5: Commit**

```bash
git add crates/legion-core/src/orchestrate/ crates/legion-core/src/lib.rs crates/legion-core/tests/orchestrate_engine_test.rs
git commit -m "feat(orchestrate): add OrchestrateEngine with worker state tracking"
```

---

### Task 2: Orchestration HTTP API

Expose the OrchestrateEngine via HTTP endpoints so CLI tools can interact with it. Follows the same hyper pattern as `ProxyControlApi` in `crates/legion-core/src/proxy/control.rs`.

**Files:**
- Create: `crates/legion-core/src/orchestrate/api.rs`
- Modify: `crates/legion-core/src/orchestrate/mod.rs`
- Modify: `crates/legion-core/src/lib.rs`
- Test: `crates/legion-core/tests/orchestrate_api_test.rs`

**Step 1: Write the failing test**

Create `crates/legion-core/tests/orchestrate_api_test.rs`:

```rust
use legion_core::orchestrate::{OrchestrateApi, OrchestrateEngine};
use std::sync::Arc;

async fn start_api(worker_count: u16, port: u16) -> Arc<OrchestrateEngine> {
    let engine = Arc::new(OrchestrateEngine::new(worker_count));
    let api = OrchestrateApi::new(engine.clone(), port);
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        api.start_with_signal(Some(tx)).await.unwrap();
    });
    rx.await.unwrap();
    engine
}

#[tokio::test]
async fn test_status_endpoint() {
    let _engine = start_api(2, 30080).await;
    let client = reqwest::Client::new();

    let resp = client
        .get("http://127.0.0.1:30080/legion/orchestrate/status")
        .send().await.unwrap();
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    let workers = body["workers"].as_array().unwrap();
    assert_eq!(workers.len(), 2);
    assert_eq!(workers[0]["status"], "idle");
}

#[tokio::test]
async fn test_dispatch_endpoint() {
    let engine = start_api(2, 30081).await;
    let client = reqwest::Client::new();

    let resp = client
        .post("http://127.0.0.1:30081/legion/orchestrate/dispatch")
        .json(&serde_json::json!({
            "worker_id": 1,
            "ticket": "Implement JWT auth"
        }))
        .send().await.unwrap();
    assert_eq!(resp.status(), 200);

    let status = engine.worker_status(1).await.unwrap();
    assert_eq!(status.ticket.as_deref(), Some("Implement JWT auth"));
}

#[tokio::test]
async fn test_report_endpoint() {
    let engine = start_api(2, 30082).await;
    let client = reqwest::Client::new();

    // Dispatch first
    engine.dispatch(1, "task".into()).await.unwrap();
    engine.mark_working(1).await;

    let resp = client
        .post("http://127.0.0.1:30082/legion/orchestrate/report")
        .json(&serde_json::json!({
            "worker_id": 1,
            "status": "done",
            "summary": "all tests pass"
        }))
        .send().await.unwrap();
    assert_eq!(resp.status(), 200);

    let status = engine.worker_status(1).await.unwrap();
    assert_eq!(status.summary.as_deref(), Some("all tests pass"));
}

#[tokio::test]
async fn test_stop_endpoint() {
    let engine = start_api(2, 30083).await;
    let client = reqwest::Client::new();

    engine.dispatch(1, "task".into()).await.unwrap();
    engine.mark_working(1).await;

    let resp = client
        .post("http://127.0.0.1:30083/legion/orchestrate/stop")
        .json(&serde_json::json!({ "worker_id": 1 }))
        .send().await.unwrap();
    assert_eq!(resp.status(), 200);

    let status = engine.worker_status(1).await.unwrap();
    assert_eq!(status.status, legion_core::WorkerTaskStatus::Stopped);
}

#[tokio::test]
async fn test_stop_all_endpoint() {
    let engine = start_api(3, 30084).await;
    let client = reqwest::Client::new();

    for i in 1..=3 {
        engine.dispatch(i, format!("task {}", i)).await.unwrap();
        engine.mark_working(i).await;
    }

    let resp = client
        .post("http://127.0.0.1:30084/legion/orchestrate/stop-all")
        .send().await.unwrap();
    assert_eq!(resp.status(), 200);

    for i in 1..=3 {
        let s = engine.worker_status(i).await.unwrap();
        assert_eq!(s.status, legion_core::WorkerTaskStatus::Stopped);
    }
}

#[tokio::test]
async fn test_404() {
    let _engine = start_api(1, 30085).await;
    let client = reqwest::Client::new();

    let resp = client
        .get("http://127.0.0.1:30085/legion/orchestrate/nonexistent")
        .send().await.unwrap();
    assert_eq!(resp.status(), 404);
}
```

**Step 2: Run test to verify it fails**

Run: `cargo test -p legion-core --test orchestrate_api_test 2>&1 | head -20`
Expected: FAIL (OrchestrateApi not found)

**Step 3: Write minimal implementation**

Create `crates/legion-core/src/orchestrate/api.rs`:

```rust
//! Orchestration HTTP API — mirrors ProxyControlApi pattern

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;
use http_body_util::Full;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{body::Incoming, Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tracing::{debug, error, info};

use super::engine::{OrchestrateEngine, WorkerTaskStatus};

pub struct OrchestrateApi {
    engine: Arc<OrchestrateEngine>,
    port: u16,
}

impl OrchestrateApi {
    pub fn new(engine: Arc<OrchestrateEngine>, port: u16) -> Self {
        Self { engine, port }
    }

    pub async fn start_with_signal(
        &self,
        ready_tx: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> Result<()> {
        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        let listener = TcpListener::bind(addr).await?;
        info!("Orchestrate API listening on http://{}", addr);

        if let Some(tx) = ready_tx {
            let _ = tx.send(());
        }

        let engine = self.engine.clone();

        loop {
            let (stream, remote_addr) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let engine = engine.clone();

            tokio::spawn(async move {
                let service = service_fn(move |req| {
                    let engine = engine.clone();
                    async move { handle_request(req, engine).await }
                });

                if let Err(err) = http1::Builder::new()
                    .serve_connection(io, service)
                    .await
                {
                    debug!("Error serving orchestrate connection from {}: {:?}", remote_addr, err);
                }
            });
        }
    }
}

async fn handle_request(
    req: Request<Incoming>,
    engine: Arc<OrchestrateEngine>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    debug!("Orchestrate API: {} {}", method, path);

    match (method, path.as_str()) {
        (Method::GET, "/legion/orchestrate/status") => handle_status(engine).await,
        (Method::POST, "/legion/orchestrate/dispatch") => handle_dispatch(req, engine).await,
        (Method::POST, "/legion/orchestrate/report") => handle_report(req, engine).await,
        (Method::POST, "/legion/orchestrate/stop") => handle_stop(req, engine).await,
        (Method::POST, "/legion/orchestrate/stop-all") => handle_stop_all(engine).await,
        _ => Ok(json_response(StatusCode::NOT_FOUND, r#"{"error": "not found"}"#)),
    }
}

async fn handle_status(
    engine: Arc<OrchestrateEngine>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let workers = engine.all_status().await;
    let body = serde_json::json!({ "workers": workers });
    Ok(json_response(StatusCode::OK, &body.to_string()))
}

async fn handle_dispatch(
    req: Request<Incoming>,
    engine: Arc<OrchestrateEngine>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let body = match read_body(req).await {
        Ok(b) => b,
        Err(resp) => return Ok(resp),
    };

    #[derive(serde::Deserialize)]
    struct DispatchReq {
        worker_id: u16,
        ticket: String,
    }

    match serde_json::from_slice::<DispatchReq>(&body) {
        Ok(req) => {
            match engine.dispatch(req.worker_id, req.ticket).await {
                Ok(()) => Ok(json_response(StatusCode::OK, r#"{"status": "dispatched"}"#)),
                Err(e) => Ok(json_response(
                    StatusCode::BAD_REQUEST,
                    &format!(r#"{{"error": "{}"}}"#, e),
                )),
            }
        }
        Err(e) => Ok(json_response(
            StatusCode::BAD_REQUEST,
            &format!(r#"{{"error": "invalid json: {}"}}"#, e),
        )),
    }
}

async fn handle_report(
    req: Request<Incoming>,
    engine: Arc<OrchestrateEngine>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let body = match read_body(req).await {
        Ok(b) => b,
        Err(resp) => return Ok(resp),
    };

    #[derive(serde::Deserialize)]
    struct ReportReq {
        worker_id: u16,
        status: String,
        summary: Option<String>,
    }

    match serde_json::from_slice::<ReportReq>(&body) {
        Ok(req) => {
            let status = match req.status.as_str() {
                "done" => WorkerTaskStatus::Done,
                "error" => WorkerTaskStatus::Error,
                other => {
                    return Ok(json_response(
                        StatusCode::BAD_REQUEST,
                        &format!(r#"{{"error": "invalid status: {}"}}"#, other),
                    ));
                }
            };
            match engine.report(req.worker_id, status, req.summary).await {
                Ok(()) => Ok(json_response(StatusCode::OK, r#"{"status": "ok"}"#)),
                Err(e) => Ok(json_response(
                    StatusCode::BAD_REQUEST,
                    &format!(r#"{{"error": "{}"}}"#, e),
                )),
            }
        }
        Err(e) => Ok(json_response(
            StatusCode::BAD_REQUEST,
            &format!(r#"{{"error": "invalid json: {}"}}"#, e),
        )),
    }
}

async fn handle_stop(
    req: Request<Incoming>,
    engine: Arc<OrchestrateEngine>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let body = match read_body(req).await {
        Ok(b) => b,
        Err(resp) => return Ok(resp),
    };

    #[derive(serde::Deserialize)]
    struct StopReq {
        worker_id: u16,
    }

    match serde_json::from_slice::<StopReq>(&body) {
        Ok(req) => {
            engine.stop_worker(req.worker_id).await;
            Ok(json_response(StatusCode::OK, r#"{"status": "stopped"}"#))
        }
        Err(e) => Ok(json_response(
            StatusCode::BAD_REQUEST,
            &format!(r#"{{"error": "invalid json: {}"}}"#, e),
        )),
    }
}

async fn handle_stop_all(
    engine: Arc<OrchestrateEngine>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    engine.stop_all().await;
    Ok(json_response(StatusCode::OK, r#"{"status": "all_stopped"}"#))
}

async fn read_body(req: Request<Incoming>) -> Result<bytes::Bytes, Response<Full<Bytes>>> {
    use http_body_util::BodyExt;
    req.collect()
        .await
        .map(|c| c.to_bytes())
        .map_err(|e| {
            error!("Failed to read body: {}", e);
            json_response(StatusCode::BAD_REQUEST, r#"{"error": "failed to read body"}"#)
        })
}

fn json_response(status: StatusCode, body: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap()
}
```

Update `crates/legion-core/src/orchestrate/mod.rs`:
```rust
mod engine;
mod api;

pub use engine::{OrchestrateEngine, WorkerState, WorkerTaskStatus};
pub use api::OrchestrateApi;
```

Update `crates/legion-core/src/lib.rs` to also export `OrchestrateApi`:
```rust
pub use orchestrate::{OrchestrateEngine, OrchestrateApi, WorkerState, WorkerTaskStatus};
```

**Step 4: Run test to verify it passes**

Run: `cargo test -p legion-core --test orchestrate_api_test -- --nocapture`
Expected: all 6 tests PASS

**Step 5: Commit**

```bash
git add crates/legion-core/src/orchestrate/api.rs crates/legion-core/src/orchestrate/mod.rs crates/legion-core/src/lib.rs crates/legion-core/tests/orchestrate_api_test.rs
git commit -m "feat(orchestrate): add HTTP API for dispatch/report/status/stop"
```

---

### Task 3: CLI Tools — legion-dispatch, legion-check, legion-report, legion-status, legion-stop

Standalone binaries that Leader/Worker call via bash. Each one POSTs to the Orchestration HTTP API on `base_port + 2000`. The port is discovered via `LEGION_ORCHESTRATE_PORT` env var (set by PTY spawn).

**Files:**
- Create: `crates/legion-tools/Cargo.toml`
- Create: `crates/legion-tools/src/bin/legion-dispatch.rs`
- Create: `crates/legion-tools/src/bin/legion-check.rs`
- Create: `crates/legion-tools/src/bin/legion-report.rs`
- Create: `crates/legion-tools/src/bin/legion-status.rs`
- Create: `crates/legion-tools/src/bin/legion-stop.rs`
- Modify: `Cargo.toml` (workspace members)

**Step 1: Create the crate**

Create `crates/legion-tools/Cargo.toml`:
```toml
[package]
name = "legion-tools"
version.workspace = true
edition.workspace = true

[[bin]]
name = "legion-dispatch"
path = "src/bin/legion-dispatch.rs"

[[bin]]
name = "legion-check"
path = "src/bin/legion-check.rs"

[[bin]]
name = "legion-report"
path = "src/bin/legion-report.rs"

[[bin]]
name = "legion-status"
path = "src/bin/legion-status.rs"

[[bin]]
name = "legion-stop"
path = "src/bin/legion-stop.rs"

[dependencies]
ureq.workspace = true
serde_json.workspace = true
```

Add `"crates/legion-tools"` to workspace members in root `Cargo.toml`.

**Step 2: Write legion-dispatch**

Create `crates/legion-tools/src/bin/legion-dispatch.rs`:
```rust
//! legion-dispatch <worker_id> "ticket text"
//! Leader calls this to assign a task to a Worker.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: legion-dispatch <worker_id> \"ticket text\"");
        std::process::exit(1);
    }

    let worker_id: u16 = args[1].parse().unwrap_or_else(|_| {
        eprintln!("Error: worker_id must be a number");
        std::process::exit(1);
    });
    let ticket = args[2..].join(" ");

    let port = get_orchestrate_port();
    let url = format!("http://127.0.0.1:{}/legion/orchestrate/dispatch", port);
    let body = serde_json::json!({
        "worker_id": worker_id,
        "ticket": ticket,
    });

    match ureq::post(&url)
        .header("Content-Type", "application/json")
        .send_bytes(body.to_string().as_bytes())
    {
        Ok(resp) => {
            let text = resp.into_body().read_to_string().unwrap_or_default();
            println!("Dispatched to Worker {}: {}", worker_id, text);
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

fn get_orchestrate_port() -> u16 {
    std::env::var("LEGION_ORCHESTRATE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20080) // default: 18080 + 2000
}
```

**Step 3: Write legion-check**

Create `crates/legion-tools/src/bin/legion-check.rs`:
```rust
//! legion-check
//! Leader calls this to see all Workers' status and results.

fn main() {
    let port = get_orchestrate_port();
    let url = format!("http://127.0.0.1:{}/legion/orchestrate/status", port);

    match ureq::get(&url).call() {
        Ok(resp) => {
            let text = resp.into_body().read_to_string().unwrap_or_default();
            let data: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();

            if let Some(workers) = data["workers"].as_array() {
                println!("=== Squad Status ===");
                for w in workers {
                    let id = w["worker_id"].as_u64().unwrap_or(0);
                    let status = w["status"].as_str().unwrap_or("unknown");
                    let ticket = w["ticket"].as_str().unwrap_or("-");
                    let elapsed = w["elapsed_secs"].as_u64().unwrap_or(0);
                    let summary = w["summary"].as_str().unwrap_or("");

                    let icon = match status {
                        "done" => "OK",
                        "error" => "ERR",
                        "working" => "WORKING",
                        "pending" => "PENDING",
                        "stopped" => "STOPPED",
                        _ => "IDLE",
                    };

                    let ticket_short = if ticket.len() > 40 {
                        format!("{}...", &ticket[..37])
                    } else {
                        ticket.to_string()
                    };

                    println!(
                        "  Worker {}: [{}] \"{}\" ({}s)",
                        id, icon, ticket_short, elapsed
                    );
                    if !summary.is_empty() {
                        println!("           Summary: {}", summary);
                    }
                }

                let total = workers.len();
                let done = workers.iter().filter(|w| w["status"] == "done").count();
                println!("\nCompleted: {}/{}", done, total);
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

fn get_orchestrate_port() -> u16 {
    std::env::var("LEGION_ORCHESTRATE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20080)
}
```

**Step 4: Write legion-report**

Create `crates/legion-tools/src/bin/legion-report.rs`:
```rust
//! legion-report <done|error> ["summary text"]
//! Worker calls this when task is complete or failed.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: legion-report <done|error> [\"summary\"]");
        std::process::exit(1);
    }

    let status = &args[1];
    if !["done", "error"].contains(&status.as_str()) {
        eprintln!("Error: status must be 'done' or 'error'");
        std::process::exit(1);
    }

    let summary = if args.len() > 2 {
        Some(args[2..].join(" "))
    } else {
        None
    };

    let worker_id = get_worker_id();
    let port = get_orchestrate_port();
    let url = format!("http://127.0.0.1:{}/legion/orchestrate/report", port);

    let mut body = serde_json::json!({
        "worker_id": worker_id,
        "status": status,
    });
    if let Some(s) = &summary {
        body["summary"] = serde_json::Value::String(s.clone());
    }

    match ureq::post(&url)
        .header("Content-Type", "application/json")
        .send_bytes(body.to_string().as_bytes())
    {
        Ok(_) => println!("Reported: {} (worker {})", status, worker_id),
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

fn get_orchestrate_port() -> u16 {
    std::env::var("LEGION_ORCHESTRATE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20080)
}

fn get_worker_id() -> u16 {
    std::env::var("LEGION_WORKER_ID")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}
```

**Step 5: Write legion-status**

Create `crates/legion-tools/src/bin/legion-status.rs`:
```rust
//! legion-status
//! One-line summary of all Workers (compact version of legion-check).

fn main() {
    let port = get_orchestrate_port();
    let url = format!("http://127.0.0.1:{}/legion/orchestrate/status", port);

    match ureq::get(&url).call() {
        Ok(resp) => {
            let text = resp.into_body().read_to_string().unwrap_or_default();
            let data: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();

            if let Some(workers) = data["workers"].as_array() {
                let parts: Vec<String> = workers.iter().map(|w| {
                    let id = w["worker_id"].as_u64().unwrap_or(0);
                    let status = w["status"].as_str().unwrap_or("?");
                    let icon = match status {
                        "done" => "OK",
                        "error" => "ERR",
                        "working" => "..",
                        "pending" => ">>",
                        "stopped" => "XX",
                        _ => "--",
                    };
                    format!("W{}[{}]", id, icon)
                }).collect();
                println!("{}", parts.join(" "));
            }
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

fn get_orchestrate_port() -> u16 {
    std::env::var("LEGION_ORCHESTRATE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20080)
}
```

**Step 6: Write legion-stop**

Create `crates/legion-tools/src/bin/legion-stop.rs`:
```rust
//! legion-stop <worker_id|all>
//! Leader calls this to stop a Worker or all Workers.

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: legion-stop <worker_id|all>");
        std::process::exit(1);
    }

    let port = get_orchestrate_port();

    if args[1] == "all" {
        let url = format!("http://127.0.0.1:{}/legion/orchestrate/stop-all", port);
        match ureq::post(&url).send_bytes(&[]) {
            Ok(_) => println!("All workers stopped"),
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        let worker_id: u16 = args[1].parse().unwrap_or_else(|_| {
            eprintln!("Error: worker_id must be a number or 'all'");
            std::process::exit(1);
        });
        let url = format!("http://127.0.0.1:{}/legion/orchestrate/stop", port);
        let body = serde_json::json!({ "worker_id": worker_id });
        match ureq::post(&url)
            .header("Content-Type", "application/json")
            .send_bytes(body.to_string().as_bytes())
        {
            Ok(_) => println!("Worker {} stopped", worker_id),
            Err(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
        }
    }
}

fn get_orchestrate_port() -> u16 {
    std::env::var("LEGION_ORCHESTRATE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20080)
}
```

**Step 7: Build and verify**

Run: `cargo build -p legion-tools`
Expected: 5 binaries compile successfully

**Step 8: Commit**

```bash
git add crates/legion-tools/ Cargo.toml
git commit -m "feat(tools): add CLI tools legion-dispatch/check/report/status/stop"
```

---

### Task 4: PTY Injection — Idle Detection + Task Writing

The orchestration engine needs to detect when a Worker's Claude Code is idle (showing prompt) and inject the task text. This runs as a background task in the TUI event loop.

**Files:**
- Modify: `crates/legion-tui/src/app.rs` — add `orchestrate_engine` field, `write_to_pane()` method
- Modify: `crates/legion-tui/src/pty.rs` — add `is_idle()` method on `SharedParser`, `LEGION_WORKER_ID` env var, `LEGION_ORCHESTRATE_PORT` env var
- Modify: `crates/legion-tui/src/lib.rs` — spawn injection polling task
- Modify: `crates/legion-tui/Cargo.toml` — add `legion-core` dep (already exists, just need orchestrate types)

**Step 1: Add idle detection to pty.rs**

Add to `crates/legion-tui/src/pty.rs` a function that checks the vt100 parser screen for Claude Code's prompt character:

```rust
/// Check if the PTY shows an idle Claude Code prompt.
/// Claude Code prompt typically ends with "❯ " or "> " on the last non-empty line.
pub fn is_pty_idle(parser: &SharedParser) -> bool {
    if let Ok(p) = parser.lock() {
        let screen = p.screen();
        // Check the last few rows for a prompt character
        let rows = screen.size().0;
        for row_idx in (0..rows).rev() {
            let row = screen.row_text(row_idx, 0, screen.size().1);
            let trimmed = row.trim();
            if trimmed.is_empty() {
                continue;
            }
            // Claude Code prompts: "❯ ", "> ", "$ "
            return trimmed.ends_with("❯")
                || trimmed.ends_with(">")
                || trimmed.ends_with("$")
                || trimmed.contains("❯ ");
        }
    }
    false
}
```

Also modify `PtyHandle::spawn()` to accept and set `LEGION_WORKER_ID` and `LEGION_ORCHESTRATE_PORT` env vars:

```rust
pub fn spawn(
    rows: u16,
    cols: u16,
    proxy_port: u16,
    control_port: u16,
    dangerously_skip_permissions: bool,
    worker_id: Option<u16>,
    orchestrate_port: Option<u16>,
) -> Result<Self> {
    // ... existing code ...
    if let Some(wid) = worker_id {
        cmd.env("LEGION_WORKER_ID", wid.to_string());
    }
    if let Some(op) = orchestrate_port {
        cmd.env("LEGION_ORCHESTRATE_PORT", op.to_string());
    }
    // ... rest of existing code ...
}
```

**Step 2: Add orchestrate state to App**

Modify `crates/legion-tui/src/app.rs`:

Add to `App` struct:
```rust
pub orchestrate: Option<Arc<OrchestrateEngine>>,
```

Add method:
```rust
/// Write bytes to a specific pane's PTY (for task injection)
pub fn write_to_pane(&mut self, pane_index: usize, data: &[u8]) {
    if let Some(pane) = self.panes.get_mut(pane_index) {
        if let Some(ref mut pty) = pane.pty {
            let _ = pty.write(data);
        }
    }
}
```

**Step 3: Add injection polling to lib.rs**

In `run_squad()`, create OrchestrateEngine + OrchestrateApi, then spawn a background task that polls for pending tasks and injects them:

```rust
// After creating app and adding panes:
let engine = Arc::new(OrchestrateEngine::new(worker_count));
app.orchestrate = Some(engine.clone());

// Start OrchestrateApi
let orchestrate_port = base_port + 2000;
let api = OrchestrateApi::new(engine.clone(), orchestrate_port);
let (orch_tx, orch_rx) = tokio::sync::oneshot::channel();
tokio::spawn(async move {
    if let Err(e) = api.start_with_signal(Some(orch_tx)).await {
        tracing::error!("Orchestrate API error: {}", e);
    }
});
orch_rx.await.ok();
```

Modify `run_event_loop` or add a separate polling check inside the loop:

```rust
// Inside event loop, after event handling:
// Check for pending tasks to inject into idle Workers
if let Some(ref engine) = app.orchestrate {
    let workers = engine.all_status().await;
    for ws in &workers {
        if ws.status == WorkerTaskStatus::Pending {
            let pane_idx = ws.worker_id as usize; // pane 0=leader, 1..N=workers
            if let Some(parser) = app.parser_at(pane_idx) {
                if is_pty_idle(parser) {
                    if let Some(ticket) = engine.take_pending(ws.worker_id).await {
                        // Inject: Ctrl-U (clear line) + ticket text + Enter
                        app.write_to_pane(pane_idx, b"\x15");
                        app.write_to_pane(pane_idx, ticket.as_bytes());
                        app.write_to_pane(pane_idx, b"\r");
                    }
                }
            }
        }
    }
}
```

**Step 4: Update add_pane() call sites**

Update `add_pane()` in `app.rs` and all call sites in `lib.rs` to pass `worker_id` and `orchestrate_port`.

In `run_squad()`:
```rust
let orchestrate_port = base_port + 2000;

// Leader: worker_id=None (leader doesn't report)
app.add_pane(leader_pty_rows, leader_pty_cols, leader_proxy, leader_control,
    "Leader".into(), true, None, Some(orchestrate_port));

for i in 0..worker_count {
    let proxy = base_port + i + 1;
    let control = base_port + 1000 + i + 1;
    let label = format!("Worker {}", i + 1);
    app.add_pane(worker_pty_rows, worker_pty_cols, proxy, control,
        label, true, Some(i + 1), Some(orchestrate_port));
}
```

In `run()` (single pane mode):
```rust
app.add_pane(pty_rows, pty_cols, proxy_port, control_port, "Claude Code".into(), false, None, None);
```

**Step 5: Build and verify**

Run: `cargo build -p legion-tui`
Expected: compiles successfully

**Step 6: Commit**

```bash
git add crates/legion-tui/src/app.rs crates/legion-tui/src/pty.rs crates/legion-tui/src/lib.rs
git commit -m "feat(tui): add PTY injection with idle detection for task dispatch"
```

---

### Task 5: Orchestration API Start in cmd_squad

Wire up the OrchestrateApi server alongside the existing proxy/control servers in the CLI's `cmd_squad` function.

**Files:**
- Modify: `crates/legion-cli/src/main.rs:341-408` — start OrchestrateApi in cmd_squad

**Step 1: Modify cmd_squad**

In `cmd_squad()`, after creating all proxy/control servers and before calling `run_squad()`, create and start the OrchestrateApi:

```rust
async fn cmd_squad(workers: u16, base_port: u16) -> Result<()> {
    // ... existing proxy/control server creation ...

    // Start Orchestrate API on base_port + 2000
    let orchestrate_port = base_port + 2000;
    let engine = std::sync::Arc::new(legion_core::OrchestrateEngine::new(workers));
    let orch_api = legion_core::OrchestrateApi::new(engine, orchestrate_port);
    let (orch_tx, orch_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        if let Err(e) = orch_api.start_with_signal(Some(orch_tx)).await {
            tracing::error!("Orchestrate API error on port {}: {}", orchestrate_port, e);
        }
    });
    ready_rxs.push(orch_rx);

    // ... wait for all servers ...

    // Run the squad TUI (pass orchestrate_port so TUI can create its own engine reference)
    legion_tui::run_squad(workers, base_port).await?;

    Ok(())
}
```

Note: The TUI creates its own `OrchestrateEngine` + `OrchestrateApi` internally (in Task 4). We may need to decide whether the engine lives in CLI or TUI. Since the TUI needs direct access to the engine for PTY injection, it's better to let the TUI own both. Remove the CLI-side orchestrate API creation and instead let `run_squad()` handle it entirely.

**Alternative (preferred):** Don't start OrchestrateApi in cmd_squad. Instead, the TUI's `run_squad()` owns the engine and starts the API. This avoids engine duplication.

The CLI just needs to pass `orchestrate_port` info to `run_squad()`:

Update `run_squad` signature:
```rust
pub async fn run_squad(worker_count: u16, base_port: u16) -> Result<()>
```

The orchestrate port is calculated inside `run_squad()` as `base_port + 2000`.

**Step 2: Build and verify**

Run: `cargo build -p legion-cli`
Expected: compiles successfully

**Step 3: Commit**

```bash
git add crates/legion-cli/src/main.rs crates/legion-tui/src/lib.rs
git commit -m "feat(cli): wire orchestration API into squad mode startup"
```

---

### Task 6: TUI Dashboard — Worker Status in Pane Borders

Show Worker task status (idle/working/done/error) and elapsed time in each pane's border title.

**Files:**
- Modify: `crates/legion-tui/src/ui.rs:132-167` — update `draw_pane()` to show orchestration status
- Modify: `crates/legion-tui/src/app.rs` — add helper to get worker status for a pane

**Step 1: Add helper to App**

In `crates/legion-tui/src/app.rs`, add:

```rust
/// Get orchestration status for a Worker pane (pane index → worker_id mapping)
pub fn worker_task_status(&self, pane_index: usize) -> Option<WorkerState> {
    // Pane 0 = Leader (no worker status), Pane 1 = Worker 1, etc.
    if pane_index == 0 || !self.is_squad() {
        return None;
    }
    let worker_id = pane_index as u16; // Worker 1 = pane 1, Worker 2 = pane 2, etc.
    // We can't call async from sync context, so we use a cached snapshot
    // Updated in the event loop
    self.orchestrate_snapshot.as_ref()
        .and_then(|snap| snap.iter().find(|w| w.worker_id == worker_id).cloned())
}
```

Add to `App` struct:
```rust
pub orchestrate_snapshot: Option<Vec<WorkerState>>,
```

In the event loop, before rendering, update the snapshot:
```rust
if let Some(ref engine) = app.orchestrate {
    app.orchestrate_snapshot = Some(engine.all_status().await);
}
```

**Step 2: Update draw_pane() in ui.rs**

Modify `draw_pane()` to show status in title when orchestration is active:

```rust
fn draw_pane(frame: &mut Frame, app: &App, index: usize, area: Rect) {
    let pane = match app.panes.get(index) {
        Some(p) => p,
        None => return,
    };

    let is_focused = app.focused_pane == index;
    let border_color = if is_focused { Color::Blue } else { Color::DarkGray };

    let title = if app.is_squad() {
        let model = pane.current_model.as_deref().unwrap_or("--");
        if let Some(ws) = app.worker_task_status(index) {
            let icon = match ws.status {
                WorkerTaskStatus::Working => "🔄",
                WorkerTaskStatus::Done => "✅",
                WorkerTaskStatus::Error => "❌",
                WorkerTaskStatus::Pending => "⏳",
                WorkerTaskStatus::Stopped => "⏸️",
                WorkerTaskStatus::Idle => "💤",
            };
            let ticket_short = ws.ticket.as_deref()
                .map(|t| if t.len() > 20 { format!("{}...", &t[..17]) } else { t.to_string() })
                .unwrap_or_default();
            let elapsed = format_elapsed(ws.elapsed_secs);
            format!(" {} | {} {} {} [{}] ", pane.label, model, icon, ticket_short, elapsed)
        } else {
            format!(" {} | {} ", pane.label, model)
        }
    } else {
        " Claude Code ".to_string()
    };

    // ... rest unchanged ...
}

fn format_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else {
        format!("{}m {}s", secs / 60, secs % 60)
    }
}
```

**Step 3: Build and verify visually**

Run: `cargo build -p legion-tui`
Expected: compiles successfully

**Step 4: Commit**

```bash
git add crates/legion-tui/src/ui.rs crates/legion-tui/src/app.rs
git commit -m "feat(tui): show worker task status in pane borders"
```

---

### Task 7: TUI Dashboard — Ctrl+T Completion Overlay

Add a toggle-able overlay showing all tickets and their status, plus footer progress summary.

**Files:**
- Modify: `crates/legion-tui/src/app.rs` — add `show_dashboard: bool` field
- Modify: `crates/legion-tui/src/ui.rs` — add `draw_dashboard_overlay()`, update footer
- Modify: `crates/legion-tui/src/input.rs` — handle Ctrl+T

**Step 1: Add dashboard toggle to App**

In `crates/legion-tui/src/app.rs`, add to `App` struct:
```rust
pub show_dashboard: bool,
```

Initialize as `false` in `App::new()`.

**Step 2: Handle Ctrl+T in input.rs**

In `handle_normal_mode()` (line ~26-65), add before the squad shortcuts:

```rust
// Ctrl+T toggles dashboard overlay (squad only)
if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('t') {
    if app.is_squad() {
        app.show_dashboard = !app.show_dashboard;
        return InputResult::Continue;
    }
}
```

**Step 3: Draw dashboard overlay in ui.rs**

Add to `draw()`, after popup overlay check:

```rust
if app.show_dashboard {
    draw_dashboard_overlay(frame, app);
}
```

Implement `draw_dashboard_overlay()`:

```rust
fn draw_dashboard_overlay(frame: &mut Frame, app: &App) {
    let area = centered_rect(70, 50, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(" Squad Progress [Ctrl+T: close] ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::DarkGray));

    let mut items: Vec<ListItem> = Vec::new();

    // Header
    items.push(ListItem::new(Line::from(vec![
        Span::styled("  #   ", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        Span::styled("Status  ", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        Span::styled("Task                              ", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        Span::styled("Worker    ", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
        Span::styled("Time", Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)),
    ])));

    if let Some(ref snapshot) = app.orchestrate_snapshot {
        let mut total = 0;
        let mut done = 0;
        for (i, ws) in snapshot.iter().enumerate() {
            total += 1;
            let (icon, color) = match ws.status {
                WorkerTaskStatus::Done => { done += 1; ("OK ", Color::Green) }
                WorkerTaskStatus::Error => ("ERR", Color::Red),
                WorkerTaskStatus::Working => ("..", Color::Yellow),
                WorkerTaskStatus::Pending => (">>", Color::Cyan),
                WorkerTaskStatus::Stopped => ("XX", Color::DarkGray),
                WorkerTaskStatus::Idle => ("--", Color::DarkGray),
            };

            let ticket = ws.ticket.as_deref().unwrap_or("-");
            let ticket_display = if ticket.len() > 32 {
                format!("{}...", &ticket[..29])
            } else {
                format!("{:<32}", ticket)
            };

            let elapsed = format_elapsed(ws.elapsed_secs);

            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("  #{:<3}", i + 1), Style::default().fg(Color::White)),
                Span::styled(format!("[{}]  ", icon), Style::default().fg(color)),
                Span::styled(ticket_display, Style::default().fg(Color::White)),
                Span::styled(format!("Worker {:<3}", ws.worker_id), Style::default().fg(Color::DarkGray)),
                Span::styled(elapsed, Style::default().fg(Color::DarkGray)),
            ])));
        }

        items.push(ListItem::new(Line::from(Span::raw(""))));
        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                format!("  Progress: {}/{} complete", done, total),
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ),
        ])));
    } else {
        items.push(ListItem::new(Line::from(
            Span::styled("  No orchestration active", Style::default().fg(Color::DarkGray))
        )));
    }

    frame.render_widget(List::new(items).block(block), area);
}
```

**Step 4: Update footer for squad progress**

In `draw_footer()`, update the squad-mode Normal hint to include progress:

```rust
if app.is_squad() && app.mode == AppMode::Normal {
    let progress = app.orchestrate_snapshot.as_ref().map(|snap| {
        let total = snap.len();
        let done = snap.iter().filter(|w| w.status == WorkerTaskStatus::Done).count();
        format!("Workers: {}/{}", done, total)
    }).unwrap_or_default();

    vec![
        Span::styled(&format!(" {}", progress), Style::default().fg(Color::Cyan)),
        Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
        Span::styled("Tab", Style::default().fg(Color::Yellow)),
        Span::styled(": Focus ", Style::default().fg(Color::DarkGray)),
        Span::styled("│ ", Style::default().fg(Color::DarkGray)),
        Span::styled("Ctrl+T", Style::default().fg(Color::Yellow)),
        Span::styled(": Dashboard ", Style::default().fg(Color::DarkGray)),
        Span::styled("│ ", Style::default().fg(Color::DarkGray)),
        Span::styled("Ctrl+P", Style::default().fg(Color::Yellow)),
        Span::styled(": Menu ", Style::default().fg(Color::DarkGray)),
        Span::styled("│ ", Style::default().fg(Color::DarkGray)),
        Span::styled("Ctrl+Q", Style::default().fg(Color::Yellow)),
        Span::styled(": Quit", Style::default().fg(Color::DarkGray)),
    ]
}
```

**Step 5: Build and verify**

Run: `cargo build -p legion-tui`
Expected: compiles successfully

**Step 6: Commit**

```bash
git add crates/legion-tui/src/app.rs crates/legion-tui/src/ui.rs crates/legion-tui/src/input.rs
git commit -m "feat(tui): add Ctrl+T dashboard overlay and footer progress"
```

---

### Task 8: Auto-Generated CLAUDE.md for Leader and Workers

When squad mode starts, generate CLAUDE.md files in a temp directory that configure Leader and Worker roles. These are injected as initial context.

**Files:**
- Create: `crates/legion-tui/src/claudemd.rs` — generate CLAUDE.md content
- Modify: `crates/legion-tui/src/lib.rs` — add `pub mod claudemd;`
- Modify: `crates/legion-tui/src/pty.rs` — accept optional `claude_md_path` and pass as `--append-system-prompt` or write to cwd

**Step 1: Write CLAUDE.md generator**

Create `crates/legion-tui/src/claudemd.rs`:

```rust
//! Auto-generate CLAUDE.md files for Leader and Worker roles

use std::fs;
use std::path::PathBuf;

use anyhow::Result;

/// Generate the Leader's CLAUDE.md content
pub fn leader_instructions(worker_count: u16) -> String {
    format!(r#"# Squad Leader

You coordinate a team of {} autonomous Workers.

## Workflow
1. Receive task from user
2. Analyze and create implementation plan
3. Split into tickets (one per worker)
4. Dispatch with: `legion-dispatch <worker_id> "ticket content"`
5. Include in each ticket:
   - Clear task description
   - Test success criteria
   - Relevant file paths and context
6. Monitor with: `legion-check`
7. When all Workers complete, verify integration
8. Report results to user

## Tools
- `legion-dispatch <id> "ticket"` — Send task to Worker
- `legion-check` — View all Workers' status and results
- `legion-status` — Quick one-line status summary
- `legion-stop <id>` / `legion-stop all` — Emergency stop

## Important
- Workers are AUTONOMOUS. Do NOT expect replies from them.
- Each Worker will execute independently using TDD.
- Use `legion-check` to poll for completion — it does not interrupt your work.
- If a Worker reports an error, decide whether to reassign, modify, or abort.
"#, worker_count)
}

/// Generate a Worker's CLAUDE.md content
pub fn worker_instructions(worker_id: u16) -> String {
    format!(r#"# Worker {} — Autonomous Task Executor

You are an autonomous worker. Execute the assigned task using TDD:

1. Read the task description carefully
2. Implement the code
3. Write tests matching the success criteria
4. Run tests until all pass
5. When complete, run: `legion-report done "brief summary of what was done"`
6. If you encounter an unrecoverable error, run: `legion-report error "description of the error"`

## Rules
- Do NOT ask for clarification. Make reasonable decisions.
- Do NOT wait for instructions. Execute immediately.
- Focus ONLY on your assigned task.
- Test thoroughly before reporting done.
"#, worker_id)
}

/// Write CLAUDE.md files to a temp directory, return paths
pub fn write_squad_claude_md(worker_count: u16) -> Result<(PathBuf, Vec<PathBuf>)> {
    let dir = PathBuf::from("/tmp/legion/claudemd");
    fs::create_dir_all(&dir)?;

    let leader_path = dir.join("leader-CLAUDE.md");
    fs::write(&leader_path, leader_instructions(worker_count))?;

    let mut worker_paths = Vec::new();
    for i in 1..=worker_count {
        let path = dir.join(format!("worker-{}-CLAUDE.md", i));
        fs::write(&path, worker_instructions(i))?;
        worker_paths.push(path);
    }

    Ok((leader_path, worker_paths))
}
```

**Step 2: Inject CLAUDE.md content via PTY**

Rather than complex CLI flag injection, we inject the CLAUDE.md content as the first message after Claude Code starts up. This is simpler and more reliable.

In `run_squad()` in `lib.rs`, after panes are added and event loop starts, we'll let the injection polling handle this naturally — the first "task" for each Worker is actually a system prompt followed by the real task.

Alternatively, modify `PtyHandle::spawn()` to set a `CLAUDE_CODE_SYSTEM_PROMPT` env var (if Claude Code supports it), or simply write the CLAUDE.md to the working directory.

The simplest approach: write CLAUDE.md to a per-worker temp directory that becomes the worker's CWD. But Claude Code reads CLAUDE.md from the project directory. So instead, we prepend the role instructions to each dispatched ticket.

In `OrchestrateEngine::dispatch()`, prepend worker instructions:

Actually, this is better handled in the dispatch flow. When Leader dispatches a ticket, the orchestration engine prepends the Worker CLAUDE.md content. Update `engine.rs`:

```rust
pub async fn dispatch_with_context(&self, worker_id: u16, ticket: String, context_prefix: Option<String>) -> Result<()> {
    let full_ticket = if let Some(prefix) = context_prefix {
        format!("{}\n\n---\n\n{}", prefix, ticket)
    } else {
        ticket
    };
    self.dispatch(worker_id, full_ticket).await
}
```

But actually, the Leader should just include context in the ticket. The CLAUDE.md is better injected as the very first thing sent to each Worker pane after startup.

**Step 3: Build and verify**

Run: `cargo build -p legion-tui`
Expected: compiles successfully

**Step 4: Commit**

```bash
git add crates/legion-tui/src/claudemd.rs crates/legion-tui/src/lib.rs
git commit -m "feat(tui): add auto-generated CLAUDE.md for Leader and Worker roles"
```

---

### Task 9: Result File Management

Workers write result summaries to `/tmp/legion/results/worker-{N}.md`. The orchestration engine manages these files, and `legion-check` reads them.

**Files:**
- Modify: `crates/legion-core/src/orchestrate/engine.rs` — add result file writing on report
- Modify: `crates/legion-tools/src/bin/legion-report.rs` — write result file before API call
- Modify: `crates/legion-tools/src/bin/legion-check.rs` — read result files

**Step 1: Add result file writing to legion-report**

In `crates/legion-tools/src/bin/legion-report.rs`, before the HTTP POST, write the result file:

```rust
// Write result file
let worker_id = get_worker_id();
let result_dir = std::path::PathBuf::from("/tmp/legion/results");
let _ = std::fs::create_dir_all(&result_dir);
let result_path = result_dir.join(format!("worker-{}.md", worker_id));
let content = format!(
    "---\nworker_id: {}\nstatus: {}\n---\n\n## Summary\n\n{}\n",
    worker_id,
    status,
    summary.as_deref().unwrap_or("(no summary)")
);
let _ = std::fs::write(&result_path, &content);
```

**Step 2: Add result file reading to legion-check**

In `crates/legion-tools/src/bin/legion-check.rs`, after printing status, check for result files:

```rust
// Also show result file content if available
let result_path = format!("/tmp/legion/results/worker-{}.md", id);
if std::path::Path::new(&result_path).exists() {
    if let Ok(content) = std::fs::read_to_string(&result_path) {
        println!("           Result: {}", result_path);
    }
}
```

**Step 3: Build and verify**

Run: `cargo build -p legion-tools`
Expected: compiles successfully

**Step 4: Commit**

```bash
git add crates/legion-tools/src/bin/legion-report.rs crates/legion-tools/src/bin/legion-check.rs
git commit -m "feat(tools): add result file management to report/check"
```

---

### Task 10: Integration — Wire Everything Together + Smoke Test

Connect all components: CLI starts squad, TUI creates engine + API, Workers get tasks injected, results flow back.

**Files:**
- Modify: `crates/legion-tui/src/lib.rs` — final integration of orchestrate engine + API + injection
- Modify: `crates/legion-tui/Cargo.toml` — ensure legion-core dep covers orchestrate types

**Step 1: Final lib.rs integration**

Ensure `run_squad()` in `lib.rs`:
1. Creates `OrchestrateEngine`
2. Starts `OrchestrateApi` on `base_port + 2000`
3. Writes CLAUDE.md files
4. Injects Worker role instructions as first "prompt" after startup
5. Event loop polls for pending tasks and injects them

**Step 2: Smoke test**

Run: `cargo build --workspace`
Expected: all crates compile

Run: `cargo test --workspace`
Expected: all tests pass (engine tests, API tests, existing proxy tests)

Optionally run manually:
```
cargo run -- squad --workers 2 --base-port 18080
```
Expected: TUI launches with Leader + 2 Workers, Ctrl+T shows empty dashboard

**Step 3: Commit**

```bash
git add .
git commit -m "feat(squad): complete orchestration integration — dispatch, inject, dashboard"
```

---

## Summary

| Task | Component | Files | Tests |
|------|-----------|-------|-------|
| 1 | OrchestrateEngine core state | engine.rs, mod.rs | 8 unit tests |
| 2 | Orchestration HTTP API | api.rs | 6 integration tests |
| 3 | CLI tools (5 binaries) | legion-tools crate | build verification |
| 4 | PTY injection + idle detection | pty.rs, app.rs, lib.rs | build verification |
| 5 | CLI cmd_squad wiring | main.rs | build verification |
| 6 | Pane border status | ui.rs, app.rs | build verification |
| 7 | Ctrl+T dashboard overlay | ui.rs, input.rs, app.rs | build verification |
| 8 | Auto-generated CLAUDE.md | claudemd.rs | build verification |
| 9 | Result file management | legion-report, legion-check | build verification |
| 10 | Full integration | lib.rs | workspace build + test |
