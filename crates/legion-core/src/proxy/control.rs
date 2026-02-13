//! Control API for runtime proxy configuration management

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
use tokio::sync::RwLock;
use tracing::{debug, error, info};

use super::server::ProxyConfig;

/// Control API server for managing proxy configuration at runtime
pub struct ProxyControlApi {
    config: Arc<RwLock<ProxyConfig>>,
    control_port: u16,
}

/// Status response from the control API
#[derive(serde::Serialize)]
struct StatusResponse {
    target_url: Option<String>,
    api_format: Option<String>,
    model: Option<String>,
    configured: bool,
}

impl ProxyControlApi {
    /// Create a new control API server
    pub fn new(config: Arc<RwLock<ProxyConfig>>, control_port: u16) -> Self {
        Self {
            config,
            control_port,
        }
    }

    /// Start the control API server
    pub async fn start(&self) -> Result<()> {
        self.start_with_signal(None).await
    }

    /// Start the control API server, optionally signaling when the listener is bound
    pub async fn start_with_signal(
        &self,
        ready_tx: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> Result<()> {
        let addr = SocketAddr::from(([127, 0, 0, 1], self.control_port));
        let listener = TcpListener::bind(addr).await?;
        info!("Control API listening on http://{}", addr);

        if let Some(tx) = ready_tx {
            let _ = tx.send(());
        }

        let config = self.config.clone();

        loop {
            let (stream, remote_addr) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let config = config.clone();

            tokio::spawn(async move {
                let service = service_fn(move |req| {
                    let config = config.clone();
                    async move { handle_control_request(req, config).await }
                });

                if let Err(err) = http1::Builder::new()
                    .serve_connection(io, service)
                    .await
                {
                    debug!("Error serving control connection from {}: {:?}", remote_addr, err);
                }
            });
        }
    }
}

/// Handle an incoming control API request
async fn handle_control_request(
    req: Request<Incoming>,
    config: Arc<RwLock<ProxyConfig>>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    debug!("Control API: {} {}", method, path);

    match (method, path.as_str()) {
        (Method::GET, "/legion/status") => handle_status(config).await,
        (Method::POST, "/legion/config") => handle_update_config(req, config).await,
        (Method::GET, "/legion/providers") => handle_providers().await,
        _ => Ok(json_response(
            StatusCode::NOT_FOUND,
            r#"{"error": "not found"}"#,
        )),
    }
}

/// GET /legion/status - Return current proxy configuration
async fn handle_status(
    config: Arc<RwLock<ProxyConfig>>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let cfg = config.read().await;
    let status = StatusResponse {
        target_url: cfg.target_url.clone(),
        api_format: cfg.api_format.clone(),
        model: cfg.model.clone(),
        configured: cfg.is_configured(),
    };

    match serde_json::to_string(&status) {
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

/// POST /legion/config - Update proxy configuration
async fn handle_update_config(
    req: Request<Incoming>,
    config: Arc<RwLock<ProxyConfig>>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    use http_body_util::BodyExt;

    let body_bytes = match req.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            error!("Failed to read control request body: {}", e);
            return Ok(json_response(
                StatusCode::BAD_REQUEST,
                r#"{"error": "failed to read body"}"#,
            ));
        }
    };

    #[derive(serde::Deserialize)]
    struct ConfigUpdate {
        target_url: Option<String>,
        api_key: Option<String>,
        api_format: Option<String>,
        model: Option<String>,
    }

    match serde_json::from_slice::<ConfigUpdate>(&body_bytes) {
        Ok(update) => {
            let mut cfg = config.write().await;
            if let Some(url) = update.target_url {
                cfg.target_url = Some(url);
            }
            if let Some(key) = update.api_key {
                cfg.api_key = Some(key);
            }
            if let Some(fmt) = update.api_format {
                cfg.api_format = Some(fmt);
            }
            if let Some(model) = update.model {
                cfg.model = Some(model);
            }
            info!("Proxy config updated via control API");
            Ok(json_response(StatusCode::OK, r#"{"ok": true}"#))
        }
        Err(e) => {
            error!("Invalid config update JSON: {}", e);
            Ok(json_response(
                StatusCode::BAD_REQUEST,
                &format!(r#"{{"error": "invalid json: {}"}}"#, e),
            ))
        }
    }
}

/// GET /legion/providers - List providers from database
async fn handle_providers() -> Result<Response<Full<Bytes>>, Infallible> {
    match legion_db::open_db() {
        Ok(repo) => match repo.list_providers() {
            Ok(providers) => {
                #[derive(serde::Serialize)]
                struct ProviderInfo {
                    id: String,
                    name: String,
                    base_url: String,
                    api_format: String,
                    models: Option<Vec<String>>,
                    is_default: bool,
                }

                let infos: Vec<ProviderInfo> = providers
                    .into_iter()
                    .map(|p| ProviderInfo {
                        id: p.id,
                        name: p.name,
                        base_url: p.base_url,
                        api_format: p.api_format,
                        models: p.models,
                        is_default: p.is_default,
                    })
                    .collect();

                match serde_json::to_string(&infos) {
                    Ok(json) => Ok(json_response(StatusCode::OK, &json)),
                    Err(e) => Ok(json_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        &format!(r#"{{"error": "{}"}}"#, e),
                    )),
                }
            }
            Err(e) => Ok(json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!(r#"{{"error": "{}"}}"#, e),
            )),
        },
        Err(e) => Ok(json_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!(r#"{{"error": "database error: {}"}}"#, e),
        )),
    }
}

/// Helper to create a JSON response
fn json_response(status: StatusCode, body: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(body.to_string())))
        .unwrap()
}
