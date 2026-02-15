//! HTTP proxy server for routing requests to configured backends

use std::convert::Infallible;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{BodyExt, Either, Full, StreamBody};
use hyper::body::Frame;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{body::Incoming, Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

use super::transform::{anthropic_to_openai, openai_to_anthropic, wrap_anthropic_json_as_sse};
use crate::copilot::{self, CopilotTokenInfo};

/// A boxed stream of frames for SSE passthrough
type BoxStream = Pin<Box<dyn futures_util::Stream<Item = Result<Frame<Bytes>, Infallible>> + Send>>;

/// Response body: either fully buffered or a streaming SSE body
type ProxyBody = Either<Full<Bytes>, StreamBody<BoxStream>>;

/// Cached Copilot API token (short-lived, auto-refreshed from gho_* OAuth token)
type CopilotTokenCache = Arc<RwLock<Option<CopilotTokenInfo>>>;

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
    copilot_cache: CopilotTokenCache,
    port: u16,
}

impl ProxyServer {
    /// Create a new proxy server on the specified port
    pub fn new(port: u16) -> Self {
        Self {
            config: Arc::new(RwLock::new(ProxyConfig::new())),
            copilot_cache: Arc::new(RwLock::new(None)),
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

    /// Get a reference to the Copilot token cache
    pub fn copilot_cache_ref(&self) -> CopilotTokenCache {
        self.copilot_cache.clone()
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
        let copilot_cache = self.copilot_cache.clone();

        loop {
            let (stream, remote_addr) = listener.accept().await?;
            let io = TokioIo::new(stream);
            let config = config.clone();
            let copilot_cache = copilot_cache.clone();

            tokio::spawn(async move {
                let service = service_fn(move |req| {
                    let config = config.clone();
                    let copilot_cache = copilot_cache.clone();
                    async move { handle_request(req, config, copilot_cache).await }
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

/// Build an error response with a fully buffered body
fn error_response(status: StatusCode, body: String) -> Response<ProxyBody> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Either::Left(Full::new(Bytes::from(body))))
        .unwrap()
}

/// Handle an incoming HTTP request
async fn handle_request(
    req: Request<Incoming>,
    config: Arc<RwLock<ProxyConfig>>,
    copilot_cache: CopilotTokenCache,
) -> Result<Response<ProxyBody>, Infallible> {
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
            return Ok(error_response(
                StatusCode::SERVICE_UNAVAILABLE,
                r#"{"error": {"type": "service_unavailable", "message": "No backend configured"}}"#.into(),
            ));
        }
    };

    // For github_copilot format: resolve the short-lived API token and effective base URL
    let copilot_resolved = if proxy_config.api_format.as_deref() == Some("github_copilot") {
        match resolve_copilot_token(&proxy_config, &copilot_cache).await {
            Ok(info) => Some(info),
            Err(e) => {
                error!("Failed to resolve Copilot token: {}", e);
                return Ok(error_response(
                    StatusCode::BAD_GATEWAY,
                    format!(r#"{{"error": {{"type": "copilot_auth_error", "message": "{}"}}}}"#, e),
                ));
            }
        }
    } else {
        None
    };

    // Only proxy POST requests
    if method != Method::POST {
        return Ok(error_response(
            StatusCode::METHOD_NOT_ALLOWED,
            r#"{"error": {"type": "method_not_allowed", "message": "Only POST requests are supported"}}"#.into(),
        ));
    }

    // Collect the request body
    let body_bytes = match req.collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) => {
            error!("Failed to read request body: {}", e);
            return Ok(error_response(
                StatusCode::BAD_REQUEST,
                format!(r#"{{"error": {{"type": "bad_request", "message": "Failed to read request body: {}"}}}}"#, e),
            ));
        }
    };

    // Transform request if needed
    // Both openai_chat and github_copilot use OpenAI format
    let is_openai_compat = matches!(
        proxy_config.api_format.as_deref(),
        Some("openai_chat") | Some("github_copilot")
    );

    let (request_body, content_type) = match proxy_config.api_format.as_deref() {
        Some("openai_chat") | Some("github_copilot") => {
            match anthropic_to_openai(&body_bytes, proxy_config.model.as_deref()) {
                Ok(transformed) => {
                    // Force stream=false — we'll wrap the buffered response as Anthropic SSE
                    let final_body = match serde_json::from_slice::<serde_json::Value>(&transformed) {
                        Ok(mut v) => {
                            v["stream"] = serde_json::Value::Bool(false);
                            serde_json::to_vec(&v).unwrap_or(transformed)
                        }
                        Err(_) => transformed,
                    };
                    (final_body, "application/json")
                }
                Err(e) => {
                    error!("Failed to transform request: {}", e);
                    return Ok(error_response(
                        StatusCode::BAD_REQUEST,
                        format!(r#"{{"error": {{"type": "bad_request", "message": "Failed to transform request: {}"}}}}"#, e),
                    ));
                }
            }
        }
        _ => {
            // For anthropic/anthropic_bearer: rewrite model if override is configured
            if let Some(ref model_override) = proxy_config.model {
                match serde_json::from_slice::<serde_json::Value>(&body_bytes) {
                    Ok(mut body) => {
                        if let Some(obj) = body.as_object_mut() {
                            obj.insert("model".to_string(), serde_json::Value::String(model_override.clone()));
                        }
                        (serde_json::to_vec(&body).unwrap_or_else(|_| body_bytes.to_vec()), "application/json")
                    }
                    Err(_) => (body_bytes.to_vec(), "application/json"),
                }
            } else {
                (body_bytes.to_vec(), "application/json")
            }
        }
    };

    // Build the target URL
    // For OpenAI-compatible formats: rewrite Anthropic endpoint to OpenAI endpoint
    let is_copilot = proxy_config.api_format.as_deref() == Some("github_copilot");
    let effective_path = if is_openai_compat && path.contains("/v1/messages") {
        if is_copilot {
            // Copilot API uses /chat/completions (no /v1 prefix)
            path.replace("/v1/messages", "/chat/completions")
        } else {
            path.replace("/v1/messages", "/v1/chat/completions")
        }
    } else {
        path.clone()
    };

    // For github_copilot: use the base_url from token exchange (may differ from config)
    let effective_base_url = if let Some(ref info) = copilot_resolved {
        &info.base_url
    } else {
        target_url.as_str()
    };

    let mut full_url = format!(
        "{}/{}",
        effective_base_url.trim_end_matches('/'),
        effective_path.trim_start_matches('/')
    );
    // Deduplicate version prefix: base_url may already contain /v1
    while full_url.contains("/v1/v1") {
        full_url = full_url.replace("/v1/v1", "/v1");
    }

    debug!("Forwarding request to {}", full_url);

    // Create the outgoing request
    let client = reqwest::Client::new();
    let mut request_builder = client
        .post(&full_url)
        .header("Content-Type", content_type)
        .body(request_body);

    // Add authentication and identity headers based on target format
    match proxy_config.api_format.as_deref() {
        Some("github_copilot") => {
            // GitHub Copilot: use the resolved short-lived token, not the gho_* OAuth token
            if let Some(ref info) = copilot_resolved {
                request_builder = request_builder
                    .header("Authorization", format!("Bearer {}", info.token))
                    .header("Copilot-Integration-Id", "vscode-chat")
                    .header("Editor-Version", "vscode/1.99.0")
                    .header("Editor-Plugin-Version", "copilot-chat/0.27.0");
            }
        }
        Some("openai_chat") => {
            if let Some(api_key) = &proxy_config.api_key {
                // OpenAI-compatible: Bearer auth + OpenCode Zen identity headers
                request_builder = request_builder
                    .header("Authorization", format!("Bearer {}", api_key))
                    .header("User-Agent", "opencode/stable/1.1.63/cli")
                    .header("x-opencode-project", "legion")
                    .header("x-opencode-session", "legion-proxy")
                    .header("x-opencode-request", "legion-user")
                    .header("x-opencode-client", "cli");
            }
        }
        Some("anthropic_bearer") => {
            if let Some(api_key) = &proxy_config.api_key {
                // Anthropic protocol with Bearer auth (e.g. third-party Anthropic-compatible APIs)
                request_builder = request_builder
                    .header("Authorization", format!("Bearer {}", api_key))
                    .header("anthropic-version", "2023-06-01")
                    .header("anthropic-beta", "claude-code-20250219,interleaved-thinking-2025-05-14,fine-grained-tool-streaming-2025-05-14")
                    .header("User-Agent", "opencode/stable/1.1.63/cli")
                    .header("x-opencode-project", "legion")
                    .header("x-opencode-session", "legion-proxy")
                    .header("x-opencode-request", "legion-user")
                    .header("x-opencode-client", "cli");
            }
        }
        _ => {
            if let Some(api_key) = &proxy_config.api_key {
                // Standard Anthropic — x-api-key auth
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
            return Ok(error_response(
                StatusCode::BAD_GATEWAY,
                format!(r#"{{"error": {{"type": "upstream_error", "message": "Failed to connect to backend: {}"}}}}"#, e),
            ));
        }
    };

    let status = response.status();
    let upstream_content_type = response.headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    let is_sse = upstream_content_type.contains("text/event-stream");

    debug!("Received {} response from upstream (sse={})", status, is_sse);

    // --- SSE passthrough for anthropic/anthropic_bearer streaming responses ---
    if is_sse && !is_openai_compat {
        let stream = response.bytes_stream().map(|result| {
            match result {
                Ok(bytes) => Ok::<_, Infallible>(Frame::data(bytes)),
                Err(e) => {
                    warn!("SSE streaming chunk error: {}", e);
                    Ok(Frame::data(Bytes::new()))
                }
            }
        });
        let boxed: BoxStream = Box::pin(stream);

        return Ok(Response::builder()
            .status(status)
            .header("Content-Type", "text/event-stream")
            .header("Cache-Control", "no-cache")
            .body(Either::Right(StreamBody::new(boxed)))
            .unwrap());
    }

    // --- Buffered response path ---
    let response_body = match response.bytes().await {
        Ok(bytes) => bytes,
        Err(e) => {
            error!("Failed to read response body: {}", e);
            return Ok(error_response(
                StatusCode::BAD_GATEWAY,
                format!(r#"{{"error": {{"type": "upstream_error", "message": "Failed to read backend response: {}"}}}}"#, e),
            ));
        }
    };

    // For openai_chat: transform OpenAI JSON → Anthropic JSON → Anthropic SSE
    if is_openai_compat && status.is_success() {
        match openai_to_anthropic(&response_body) {
            Ok(anthropic_json) => {
                match wrap_anthropic_json_as_sse(&anthropic_json) {
                    Ok(sse_bytes) => {
                        return Ok(Response::builder()
                            .status(status)
                            .header("Content-Type", "text/event-stream")
                            .header("Cache-Control", "no-cache")
                            .body(Either::Left(Full::new(Bytes::from(sse_bytes))))
                            .unwrap());
                    }
                    Err(e) => {
                        warn!("Failed to wrap response as SSE, returning JSON: {}", e);
                        return Ok(Response::builder()
                            .status(status)
                            .header("Content-Type", "application/json")
                            .body(Either::Left(Full::new(Bytes::from(anthropic_json))))
                            .unwrap());
                    }
                }
            }
            Err(e) => {
                warn!("Failed to transform OpenAI response, returning as-is: {}", e);
            }
        }
    }

    // Default: pass through the buffered response
    Ok(Response::builder()
        .status(status)
        .header("Content-Type", upstream_content_type)
        .body(Either::Left(Full::new(response_body)))
        .unwrap())
}

/// Resolve a valid Copilot API token, refreshing from the gho_* OAuth token if expired.
///
/// When token exchange fails (e.g. 404 for certain token types), falls back to using
/// the gho_* token directly as the API token (same behavior as OpenCode/ccswitch).
async fn resolve_copilot_token(
    config: &ProxyConfig,
    cache: &CopilotTokenCache,
) -> anyhow::Result<CopilotTokenInfo> {
    // Check if cached token is still valid (with 60s buffer)
    {
        let cached = cache.read().await;
        if let Some(ref info) = *cached {
            if info.expires_at > Instant::now() + Duration::from_secs(60) {
                return Ok(info.clone());
            }
            debug!("Copilot token expired, refreshing...");
        }
    }

    // Need to exchange/refresh
    let gho_token = config
        .api_key
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("No GitHub OAuth token (gho_*) configured for Copilot"))?;

    let info = match copilot::exchange_copilot_token(gho_token).await {
        Ok(info) => {
            info!("Copilot token exchanged, base_url: {}, expires in {}s",
                info.base_url,
                info.expires_at.duration_since(Instant::now()).as_secs()
            );
            info
        }
        Err(e) => {
            // Fallback: use gho_* token directly (some token types don't support exchange)
            warn!("Copilot token exchange failed ({}), using gho token directly", e);
            CopilotTokenInfo {
                token: gho_token.to_string(),
                base_url: copilot::DEFAULT_COPILOT_API_BASE.to_string(),
                expires_at: Instant::now() + Duration::from_secs(60 * 60),
            }
        }
    };

    // Cache the resolved token
    let mut cached = cache.write().await;
    *cached = Some(info.clone());

    Ok(info)
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
