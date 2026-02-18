//! HTTP API for the orchestration engine

use std::convert::Infallible;
use std::net::SocketAddr;

use anyhow::Result;
use bytes::Bytes;
use http_body_util::Full;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{body::Incoming, Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tracing::{debug, error, info};

use super::engine::OrchestrateEngine;

/// HTTP API server that exposes the OrchestrateEngine to CLI tools.
pub struct OrchestrateApi {
    engine: OrchestrateEngine,
    port: u16,
}

impl OrchestrateApi {
    /// Create a new orchestrate API server.
    pub fn new(engine: OrchestrateEngine, port: u16) -> Self {
        Self { engine, port }
    }

    /// Start the API server, optionally signaling when the listener is bound.
    /// If `shutdown_rx` is provided, the server will stop when it receives a signal.
    pub async fn start_with_signal(
        &self,
        ready_tx: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> Result<()> {
        self.start_with_shutdown(ready_tx, None).await
    }

    /// Start the API server with an optional shutdown signal.
    pub async fn start_with_shutdown(
        &self,
        ready_tx: Option<tokio::sync::oneshot::Sender<()>>,
        mut shutdown_rx: Option<tokio::sync::oneshot::Receiver<()>>,
    ) -> Result<()> {
        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        let listener = TcpListener::bind(addr).await?;
        info!("Orchestrate API listening on http://{}", addr);

        if let Some(tx) = ready_tx {
            let _ = tx.send(());
        }

        let engine = self.engine.clone();

        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    let (stream, remote_addr) = accept_result?;
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
                            debug!(
                                "Error serving orchestrate connection from {}: {:?}",
                                remote_addr, err
                            );
                        }
                    });
                }
                _ = async {
                    match shutdown_rx.as_mut() {
                        Some(rx) => { let _ = rx.await; },
                        None => std::future::pending::<()>().await,
                    }
                } => {
                    info!("Orchestrate API on port {} shutting down", self.port);
                    break;
                }
            }
        }
        Ok(())
    }
}

/// Route an incoming request to the correct handler.
async fn handle_request(
    req: Request<Incoming>,
    engine: OrchestrateEngine,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    debug!("Orchestrate API: {} {}", method, path);

    match (method, path.as_str()) {
        (Method::GET, "/legion/orchestrate/status") => handle_status(engine).await,
        (Method::POST, "/legion/orchestrate/submit") => handle_submit(req, engine).await,
        (Method::POST, "/legion/orchestrate/dispatch") => handle_dispatch_compat(req, engine).await,
        (Method::POST, "/legion/orchestrate/report") => handle_report(req, engine).await,
        (Method::POST, "/legion/orchestrate/stop-all") => handle_stop_all(engine).await,
        _ => Ok(json_response(
            StatusCode::NOT_FOUND,
            r#"{"error": "not found"}"#,
        )),
    }
}

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
            return Ok(json_response(
                StatusCode::BAD_REQUEST,
                r#"{"error": "failed to read body"}"#,
            ));
        }
    };

    #[derive(serde::Deserialize)]
    struct SubmitRequest {
        title: String,
        ticket: String,
        #[serde(default)]
        context: Option<String>,
        #[serde(default)]
        criteria: Option<String>,
        #[serde(default)]
        team_mode: Option<super::engine::TeamMode>,
        #[serde(default)]
        max_iterations: Option<u16>,
    }

    match serde_json::from_slice::<SubmitRequest>(&body_bytes) {
        Ok(req) => {
            let mode = req.team_mode.unwrap_or_default();
            let max_iter = match req.max_iterations {
                Some(n) if n > 0 => n,
                _ => engine.default_max_iterations().await,
            };
            let id = match engine
                .submit_ticket(req.title, req.ticket, req.context, req.criteria, mode, max_iter, Vec::new())
                .await {
                    Ok(id) => id,
                    Err(e) => return Ok(json_response(StatusCode::BAD_REQUEST, &format!(r#"{{"error": "{}"}}"#, e))),
                };
            Ok(json_response(
                StatusCode::OK,
                &format!(r#"{{"ticket_id": {}}}"#, id),
            ))
        }
        Err(e) => {
            error!("Invalid submit JSON: {}", e);
            Ok(json_response(
                StatusCode::BAD_REQUEST,
                &format!(r#"{{"error": "{}"}}"#, e),
            ))
        }
    }
}

/// Backward compat: dispatch assigns ticket to queue (ignores worker_id).
async fn handle_dispatch_compat(
    req: Request<Incoming>,
    engine: OrchestrateEngine,
) -> Result<Response<Full<Bytes>>, Infallible> {
    use http_body_util::BodyExt;

    let body_bytes = match req.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(_) => {
            return Ok(json_response(
                StatusCode::BAD_REQUEST,
                r#"{"error": "bad body"}"#,
            ))
        }
    };

    #[derive(serde::Deserialize)]
    struct DispatchRequest {
        ticket: String,
        #[serde(default)]
        #[allow(dead_code)]
        worker_id: Option<u16>,
    }

    match serde_json::from_slice::<DispatchRequest>(&body_bytes) {
        Ok(req) => {
            let title = if req.ticket.len() > 40 {
                format!("{}...", &req.ticket[..37])
            } else {
                req.ticket.clone()
            };
            let id = match engine
                .submit_ticket(title, req.ticket, None, None, super::engine::TeamMode::default(), 5, Vec::new())
                .await {
                    Ok(id) => id,
                    Err(e) => return Ok(json_response(StatusCode::BAD_REQUEST, &format!(r#"{{"error": "{}"}}"#, e))),
                };
            Ok(json_response(
                StatusCode::OK,
                &format!(
                    r#"{{"ticket_id": {}, "status": "dispatched"}}"#,
                    id
                ),
            ))
        }
        Err(e) => Ok(json_response(
            StatusCode::BAD_REQUEST,
            &format!(r#"{{"error": "{}"}}"#, e),
        )),
    }
}

/// GET /legion/orchestrate/status — return all ticket states and queue stats.
async fn handle_status(engine: OrchestrateEngine) -> Result<Response<Full<Bytes>>, Infallible> {
    let tickets = engine.all_tickets().await;
    let (total, queued, working, done, error_count) = engine.queue_stats().await;

    #[derive(serde::Serialize)]
    struct StatusResponse {
        tickets: Vec<super::engine::TicketSnapshot>,
        total: usize,
        queued: usize,
        working: usize,
        done: usize,
        error: usize,
    }

    let resp = StatusResponse {
        tickets,
        total,
        queued,
        working,
        done,
        error: error_count,
    };
    match serde_json::to_string(&resp) {
        Ok(json) => Ok(json_response(StatusCode::OK, &json)),
        Err(e) => {
            error!("Failed to serialize status: {}", e);
            Ok(json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                r#"{"error": "serialization failed"}"#,
            ))
        }
    }
}

/// POST /legion/orchestrate/report — worker reports iteration completion status.
async fn handle_report(
    req: Request<Incoming>,
    engine: OrchestrateEngine,
) -> Result<Response<Full<Bytes>>, Infallible> {
    use http_body_util::BodyExt;

    let body_bytes = match req.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            error!("Failed to read report body: {}", e);
            return Ok(json_response(
                StatusCode::BAD_REQUEST,
                r#"{"error": "failed to read body"}"#,
            ));
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
            if let Some(ticket) = engine.worker_ticket(req.worker_id).await {
                let _should_retry =
                    engine.report_iteration(ticket.id, success, req.summary).await;
                Ok(json_response(StatusCode::OK, r#"{"status": "ok"}"#))
            } else {
                Ok(json_response(
                    StatusCode::BAD_REQUEST,
                    r#"{"error": "worker has no active ticket"}"#,
                ))
            }
        }
        Err(e) => {
            error!("Invalid report JSON: {}", e);
            Ok(json_response(
                StatusCode::BAD_REQUEST,
                &format!(r#"{{"error": "{}"}}"#, e),
            ))
        }
    }
}

/// POST /legion/orchestrate/stop-all — stop all active workers.
async fn handle_stop_all(engine: OrchestrateEngine) -> Result<Response<Full<Bytes>>, Infallible> {
    engine.stop_all().await;
    Ok(json_response(
        StatusCode::OK,
        r#"{"status": "all_stopped"}"#,
    ))
}

/// Helper to create a JSON response with the correct content-type header.
fn json_response(status: StatusCode, body: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap()
}
