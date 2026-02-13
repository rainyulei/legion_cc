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

use super::engine::{OrchestrateEngine, WorkerTaskStatus};

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
                    debug!(
                        "Error serving orchestrate connection from {}: {:?}",
                        remote_addr, err
                    );
                }
            });
        }
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
        (Method::POST, "/legion/orchestrate/dispatch") => handle_dispatch(req, engine).await,
        (Method::POST, "/legion/orchestrate/report") => handle_report(req, engine).await,
        (Method::POST, "/legion/orchestrate/stop") => handle_stop(req, engine).await,
        (Method::POST, "/legion/orchestrate/stop-all") => handle_stop_all(engine).await,
        _ => Ok(json_response(
            StatusCode::NOT_FOUND,
            r#"{"error": "not found"}"#,
        )),
    }
}

/// GET /legion/orchestrate/status — return all worker states.
async fn handle_status(engine: OrchestrateEngine) -> Result<Response<Full<Bytes>>, Infallible> {
    let workers = engine.all_status().await;

    #[derive(serde::Serialize)]
    struct StatusResponse {
        workers: Vec<super::engine::WorkerState>,
    }

    let resp = StatusResponse { workers };
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

/// POST /legion/orchestrate/dispatch — assign a ticket to a worker.
async fn handle_dispatch(
    req: Request<Incoming>,
    engine: OrchestrateEngine,
) -> Result<Response<Full<Bytes>>, Infallible> {
    use http_body_util::BodyExt;

    let body_bytes = match req.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            error!("Failed to read dispatch request body: {}", e);
            return Ok(json_response(
                StatusCode::BAD_REQUEST,
                r#"{"error": "failed to read body"}"#,
            ));
        }
    };

    #[derive(serde::Deserialize)]
    struct DispatchRequest {
        worker_id: u16,
        ticket: String,
    }

    match serde_json::from_slice::<DispatchRequest>(&body_bytes) {
        Ok(req) => match engine.dispatch(req.worker_id, req.ticket).await {
            Ok(()) => Ok(json_response(StatusCode::OK, r#"{"status": "dispatched"}"#)),
            Err(e) => Ok(json_response(
                StatusCode::BAD_REQUEST,
                &format!(r#"{{"error": "{}"}}"#, e),
            )),
        },
        Err(e) => {
            error!("Invalid dispatch JSON: {}", e);
            Ok(json_response(
                StatusCode::BAD_REQUEST,
                &format!(r#"{{"error": "invalid json: {}"}}"#, e),
            ))
        }
    }
}

/// POST /legion/orchestrate/report — worker reports completion status.
async fn handle_report(
    req: Request<Incoming>,
    engine: OrchestrateEngine,
) -> Result<Response<Full<Bytes>>, Infallible> {
    use http_body_util::BodyExt;

    let body_bytes = match req.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            error!("Failed to read report request body: {}", e);
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
            let task_status = match req.status.as_str() {
                "done" => WorkerTaskStatus::Done,
                "error" => WorkerTaskStatus::Error,
                other => {
                    return Ok(json_response(
                        StatusCode::BAD_REQUEST,
                        &format!(r#"{{"error": "invalid status: {}"}}"#, other),
                    ));
                }
            };

            match engine.report(req.worker_id, task_status, req.summary).await {
                Ok(()) => Ok(json_response(StatusCode::OK, r#"{"status": "ok"}"#)),
                Err(e) => Ok(json_response(
                    StatusCode::BAD_REQUEST,
                    &format!(r#"{{"error": "{}"}}"#, e),
                )),
            }
        }
        Err(e) => {
            error!("Invalid report JSON: {}", e);
            Ok(json_response(
                StatusCode::BAD_REQUEST,
                &format!(r#"{{"error": "invalid json: {}"}}"#, e),
            ))
        }
    }
}

/// POST /legion/orchestrate/stop — stop a single worker.
async fn handle_stop(
    req: Request<Incoming>,
    engine: OrchestrateEngine,
) -> Result<Response<Full<Bytes>>, Infallible> {
    use http_body_util::BodyExt;

    let body_bytes = match req.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            error!("Failed to read stop request body: {}", e);
            return Ok(json_response(
                StatusCode::BAD_REQUEST,
                r#"{"error": "failed to read body"}"#,
            ));
        }
    };

    #[derive(serde::Deserialize)]
    struct StopRequest {
        worker_id: u16,
    }

    match serde_json::from_slice::<StopRequest>(&body_bytes) {
        Ok(req) => {
            engine.stop_worker(req.worker_id).await;
            Ok(json_response(StatusCode::OK, r#"{"status": "stopped"}"#))
        }
        Err(e) => {
            error!("Invalid stop JSON: {}", e);
            Ok(json_response(
                StatusCode::BAD_REQUEST,
                &format!(r#"{{"error": "invalid json: {}"}}"#, e),
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
