//! HTTP proxy server for routing requests to configured backends

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{body::Incoming, Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use super::transform::{anthropic_to_openai, openai_to_anthropic};

/// Configuration for the proxy server
#[derive(Debug, Clone, Default)]
pub struct ProxyConfig {
    /// Target URL to forward requests to
    pub target_url: Option<String>,
    /// API key for authentication with the target
    pub api_key: Option<String>,
    /// API format: "anthropic" or "openai_chat"
    pub api_format: Option<String>,
    /// Model to use (may override request model)
    pub model: Option<String>,
}

impl ProxyConfig {
    /// Create a new empty proxy configuration
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if the proxy is configured with a target
    pub fn is_configured(&self) -> bool {
        self.target_url.is_some() && self.api_key.is_some()
    }
}

/// HTTP proxy server that intercepts and forwards requests
pub struct ProxyServer {
    config: Arc<RwLock<ProxyConfig>>,
    port: u16,
}

impl ProxyServer {
    /// Create a new proxy server on the specified port
    pub fn new(port: u16) -> Self {
        Self {
            config: Arc::new(RwLock::new(ProxyConfig::new())),
            port,
        }
    }

    /// Update the proxy configuration
    pub async fn update_config(&self, config: ProxyConfig) {
        let mut current = self.config.write().await;
        *current = config;
        info!("Proxy configuration updated");
    }

    /// Get the current proxy configuration
    pub async fn get_config(&self) -> ProxyConfig {
        self.config.read().await.clone()
    }

    /// Get a reference to the config for sharing with handlers
    pub fn config_ref(&self) -> Arc<RwLock<ProxyConfig>> {
        self.config.clone()
    }

    /// Get the port this server is configured to run on
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Start the proxy server
    pub async fn start(&self) -> Result<()> {
        self.start_with_signal(None).await
    }

    /// Start the proxy server, optionally signaling when the listener is bound
    pub async fn start_with_signal(
        &self,
        ready_tx: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> Result<()> {
        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        let listener = TcpListener::bind(addr).await?;
        info!("Proxy server listening on http://{}", addr);

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
                    async move { handle_request(req, config).await }
                });

                if let Err(err) = http1::Builder::new()
                    .serve_connection(io, service)
                    .await
                {
                    debug!("Error serving connection from {}: {:?}", remote_addr, err);
                }
            });
        }
    }
}

/// Handle an incoming HTTP request
async fn handle_request(
    req: Request<Incoming>,
    config: Arc<RwLock<ProxyConfig>>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let path = uri.path().to_string();

    debug!("Received {} request to {}", method, path);

    // Read current config
    let proxy_config = config.read().await.clone();

    // Check if we have a target configured
    let target_url = match &proxy_config.target_url {
        Some(url) => url.clone(),
        None => {
            warn!("No target URL configured, returning 503");
            return Ok(Response::builder()
                .status(StatusCode::SERVICE_UNAVAILABLE)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(
                    r#"{"error": {"type": "service_unavailable", "message": "No backend configured"}}"#,
                )))
                .unwrap());
        }
    };

    // Only proxy POST requests to the messages endpoint
    if method != Method::POST {
        return Ok(Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .header("Content-Type", "application/json")
            .body(Full::new(Bytes::from(
                r#"{"error": {"type": "method_not_allowed", "message": "Only POST requests are supported"}}"#,
            )))
            .unwrap());
    }

    // Collect the request body
    let body_bytes = match req.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            error!("Failed to read request body: {}", e);
            return Ok(Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(format!(
                    r#"{{"error": {{"type": "bad_request", "message": "Failed to read request body: {}"}}}}"#,
                    e
                ))))
                .unwrap());
        }
    };

    // Transform request if needed
    let (request_body, content_type) = match proxy_config.api_format.as_deref() {
        Some("openai_chat") => {
            match anthropic_to_openai(&body_bytes, proxy_config.model.as_deref()) {
                Ok(transformed) => (transformed, "application/json"),
                Err(e) => {
                    error!("Failed to transform request: {}", e);
                    return Ok(Response::builder()
                        .status(StatusCode::BAD_REQUEST)
                        .header("Content-Type", "application/json")
                        .body(Full::new(Bytes::from(format!(
                            r#"{{"error": {{"type": "bad_request", "message": "Failed to transform request: {}"}}}}"#,
                            e
                        ))))
                        .unwrap());
                }
            }
        }
        _ => (body_bytes.to_vec(), "application/json"),
    };

    // Build the target URL
    let full_url = if target_url.ends_with('/') {
        format!("{}{}", target_url.trim_end_matches('/'), path)
    } else {
        format!("{}{}", target_url, path)
    };

    debug!("Forwarding request to {}", full_url);

    // Create the outgoing request
    let client = reqwest::Client::new();
    let mut request_builder = client
        .post(&full_url)
        .header("Content-Type", content_type)
        .body(request_body);

    // Add API key if configured
    if let Some(api_key) = &proxy_config.api_key {
        // Use appropriate header based on target format
        match proxy_config.api_format.as_deref() {
            Some("openai_chat") => {
                request_builder = request_builder.header("Authorization", format!("Bearer {}", api_key));
            }
            Some("anthropic_bearer") => {
                // Copilot, OpenCode Zen, etc. — Anthropic format but Bearer auth
                request_builder = request_builder.header("Authorization", format!("Bearer {}", api_key));
                request_builder = request_builder.header("anthropic-version", "2023-06-01");
            }
            _ => {
                // Native Anthropic — x-api-key auth
                request_builder = request_builder.header("x-api-key", api_key);
                request_builder = request_builder.header("anthropic-version", "2023-06-01");
            }
        }
    }

    // Send the request
    let response = match request_builder.send().await {
        Ok(resp) => resp,
        Err(e) => {
            error!("Failed to forward request: {}", e);
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(format!(
                    r#"{{"error": {{"type": "upstream_error", "message": "Failed to connect to backend: {}"}}}}"#,
                    e
                ))))
                .unwrap());
        }
    };

    let status = response.status();
    debug!("Received {} response from upstream", status);

    // Get response body
    let response_body = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => {
            error!("Failed to read response body: {}", e);
            return Ok(Response::builder()
                .status(StatusCode::BAD_GATEWAY)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(format!(
                    r#"{{"error": {{"type": "upstream_error", "message": "Failed to read backend response: {}"}}}}"#,
                    e
                ))))
                .unwrap());
        }
    };

    // Transform response if needed (only for successful responses)
    let final_body = if status.is_success() && proxy_config.api_format.as_deref() == Some("openai_chat") {
        match openai_to_anthropic(&response_body) {
            Ok(transformed) => Bytes::from(transformed),
            Err(e) => {
                warn!("Failed to transform response, returning as-is: {}", e);
                response_body
            }
        }
    } else {
        response_body
    };

    // Build response
    Ok(Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(final_body))
        .unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_proxy_config_default() {
        let config = ProxyConfig::new();
        assert!(config.target_url.is_none());
        assert!(config.api_key.is_none());
        assert!(!config.is_configured());
    }

    #[tokio::test]
    async fn test_proxy_config_is_configured() {
        let mut config = ProxyConfig::new();
        config.target_url = Some("https://api.example.com".to_string());
        assert!(!config.is_configured());

        config.api_key = Some("sk-test".to_string());
        assert!(config.is_configured());
    }

    #[tokio::test]
    async fn test_proxy_server_config_update() {
        let server = ProxyServer::new(8080);

        let config = ProxyConfig {
            target_url: Some("https://api.openai.com/v1".to_string()),
            api_key: Some("sk-test".to_string()),
            api_format: Some("openai_chat".to_string()),
            model: Some("gpt-4".to_string()),
        };

        server.update_config(config.clone()).await;

        let retrieved = server.get_config().await;
        assert_eq!(retrieved.target_url, config.target_url);
        assert_eq!(retrieved.api_key, config.api_key);
        assert_eq!(retrieved.api_format, config.api_format);
        assert_eq!(retrieved.model, config.model);
    }
}
