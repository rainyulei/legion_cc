//! Integration test for proxy SSE streaming passthrough and URL construction

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{body::Incoming, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use legion_core::proxy::{ProxyControlApi, ProxyServer};
use tokio::net::TcpListener;

/// Start a mock upstream that returns an SSE response
async fn start_mock_sse_upstream(port: u16) -> tokio::sync::oneshot::Receiver<String> {
    let (url_tx, url_rx) = tokio::sync::oneshot::channel::<String>();
    let url_tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(url_tx)));

    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
            .await
            .unwrap();
        let _ = ready_tx.send(());

        let (stream, _) = listener.accept().await.unwrap();
        let io = TokioIo::new(stream);
        let url_tx = url_tx.clone();

        let service = service_fn(move |req: Request<Incoming>| {
            let url_tx = url_tx.clone();
            async move {
                // Capture the full request URI for verification
                let uri = req.uri().to_string();
                if let Some(tx) = url_tx.lock().await.take() {
                    let _ = tx.send(uri);
                }

                // Read request body to check model rewriting
                let body = req.collect().await.unwrap().to_bytes();
                let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let model = body_json["model"].as_str().unwrap_or("unknown");

                // Build SSE response with the received model info
                let sse = format!(
                    "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"id\":\"msg_test\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"{model}\",\"stop_reason\":null,\"usage\":{{\"input_tokens\":10,\"output_tokens\":0}}}}}}\n\n\
                     event: content_block_start\ndata: {{\"type\":\"content_block_start\",\"index\":0,\"content_block\":{{\"type\":\"text\",\"text\":\"\"}}}}\n\n\
                     event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"Hello from {model}\"}}}}\n\n\
                     event: content_block_stop\ndata: {{\"type\":\"content_block_stop\",\"index\":0}}\n\n\
                     event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\"}},\"usage\":{{\"output_tokens\":5}}}}\n\n\
                     event: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n"
                );

                Ok::<_, std::convert::Infallible>(
                    Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "text/event-stream")
                        .header("Cache-Control", "no-cache")
                        .body(Full::new(Bytes::from(sse)))
                        .unwrap(),
                )
            }
        });

        let _ = http1::Builder::new().serve_connection(io, service).await;
    });
    ready_rx.await.unwrap();
    url_rx
}

/// Start a mock upstream that returns a JSON (non-streaming) response
async fn start_mock_json_upstream(port: u16) -> tokio::sync::oneshot::Receiver<String> {
    let (url_tx, url_rx) = tokio::sync::oneshot::channel::<String>();
    let url_tx = std::sync::Arc::new(tokio::sync::Mutex::new(Some(url_tx)));

    let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let listener = TcpListener::bind(format!("127.0.0.1:{}", port))
            .await
            .unwrap();
        let _ = ready_tx.send(());

        let (stream, _) = listener.accept().await.unwrap();
        let io = TokioIo::new(stream);
        let url_tx = url_tx.clone();

        let service = service_fn(move |req: Request<Incoming>| {
            let url_tx = url_tx.clone();
            async move {
                let uri = req.uri().to_string();
                if let Some(tx) = url_tx.lock().await.take() {
                    let _ = tx.send(uri);
                }

                // Read body to verify stream=false for openai_chat
                let body = req.collect().await.unwrap().to_bytes();
                let body_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let stream_flag = body_json["stream"].as_bool().unwrap_or(true);

                // Return OpenAI chat completion format
                let resp = serde_json::json!({
                    "id": "chatcmpl-test",
                    "object": "chat.completion",
                    "model": body_json["model"],
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": format!("Hello! stream={}", stream_flag)
                        },
                        "finish_reason": "stop"
                    }],
                    "usage": {
                        "prompt_tokens": 10,
                        "completion_tokens": 5,
                        "total_tokens": 15
                    }
                });

                Ok::<_, std::convert::Infallible>(
                    Response::builder()
                        .status(StatusCode::OK)
                        .header("Content-Type", "application/json")
                        .body(Full::new(Bytes::from(serde_json::to_vec(&resp).unwrap())))
                        .unwrap(),
                )
            }
        });

        let _ = http1::Builder::new().serve_connection(io, service).await;
    });
    ready_rx.await.unwrap();
    url_rx
}

#[tokio::test]
async fn test_sse_passthrough_anthropic_bearer() {
    // Start mock upstream
    let url_rx = start_mock_sse_upstream(28180).await;

    // Start proxy
    let proxy = ProxyServer::new(28181);
    let config_ref = proxy.config_ref();
    let (proxy_tx, proxy_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        proxy.start_with_signal(Some(proxy_tx)).await.unwrap();
    });
    proxy_rx.await.unwrap();

    // Start control API
    let (ctrl_tx, ctrl_rx) = tokio::sync::oneshot::channel();
    let control = ProxyControlApi::new(config_ref, 29181);
    tokio::spawn(async move {
        control.start_with_signal(Some(ctrl_tx)).await.unwrap();
    });
    ctrl_rx.await.unwrap();

    // Configure proxy: anthropic_bearer format pointing to mock
    let client = reqwest::Client::new();
    let _ = client
        .post("http://127.0.0.1:29181/legion/config")
        .json(&serde_json::json!({
            "target_url": "http://127.0.0.1:28180",
            "api_key": "test-key",
            "api_format": "anthropic_bearer",
            "model": "kimi-k2.5-free"
        }))
        .send()
        .await
        .unwrap();

    // Send request through proxy (as Claude Code would)
    let response = client
        .post("http://127.0.0.1:28181/v1/messages")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": "claude-opus-4-6",
            "max_tokens": 1024,
            "stream": true,
            "messages": [{"role": "user", "content": "hi"}]
        }))
        .send()
        .await
        .unwrap();

    // Verify response is SSE
    assert_eq!(response.status(), 200);
    let ct = response.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(ct.contains("text/event-stream"), "Expected SSE content-type, got: {}", ct);

    // Read full SSE body
    let body = response.text().await.unwrap();
    assert!(body.contains("message_start"), "SSE body should contain message_start: {}", body);
    assert!(body.contains("kimi-k2.5-free"), "Model should be rewritten to kimi-k2.5-free: {}", body);
    assert!(body.contains("Hello from kimi-k2.5-free"), "Response should contain mock content");

    // Verify the upstream received correct URL (no double /v1)
    let received_url = url_rx.await.unwrap();
    assert!(
        !received_url.contains("/v1/v1"),
        "URL should not have double /v1: {}",
        received_url
    );
    assert!(
        received_url.contains("/v1/messages"),
        "URL should contain /v1/messages: {}",
        received_url
    );
}

#[tokio::test]
async fn test_url_dedup_with_v1_base() {
    // Mock upstream with /v1 in base_url
    let url_rx = start_mock_sse_upstream(28280).await;

    let proxy = ProxyServer::new(28281);
    let config_ref = proxy.config_ref();
    let (proxy_tx, proxy_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        proxy.start_with_signal(Some(proxy_tx)).await.unwrap();
    });
    proxy_rx.await.unwrap();

    let (ctrl_tx, ctrl_rx) = tokio::sync::oneshot::channel();
    let control = ProxyControlApi::new(config_ref, 29281);
    tokio::spawn(async move {
        control.start_with_signal(Some(ctrl_tx)).await.unwrap();
    });
    ctrl_rx.await.unwrap();

    // Configure with /v1 in base URL (like opencode-zen)
    let client = reqwest::Client::new();
    let _ = client
        .post("http://127.0.0.1:29281/legion/config")
        .json(&serde_json::json!({
            "target_url": "http://127.0.0.1:28280/v1",
            "api_key": "test-key",
            "api_format": "anthropic_bearer",
            "model": "test-model"
        }))
        .send()
        .await
        .unwrap();

    // Send request - path is /v1/messages, base already has /v1
    let response = client
        .post("http://127.0.0.1:28281/v1/messages")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": "claude-opus-4-6",
            "max_tokens": 1024,
            "stream": true,
            "messages": [{"role": "user", "content": "test"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    // The key assertion: upstream should NOT receive /v1/v1/messages
    let received_url = url_rx.await.unwrap();
    assert!(
        !received_url.contains("/v1/v1"),
        "URL dedup failed - got double /v1: {}",
        received_url
    );
    // Should be /v1/messages?beta=true
    assert!(
        received_url.starts_with("/v1/messages"),
        "Expected /v1/messages, got: {}",
        received_url
    );
}

#[tokio::test]
async fn test_openai_chat_forced_non_stream() {
    // Mock upstream returns JSON (non-streaming) for openai_chat
    let url_rx = start_mock_json_upstream(28380).await;

    let proxy = ProxyServer::new(28381);
    let config_ref = proxy.config_ref();
    let (proxy_tx, proxy_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        proxy.start_with_signal(Some(proxy_tx)).await.unwrap();
    });
    proxy_rx.await.unwrap();

    let (ctrl_tx, ctrl_rx) = tokio::sync::oneshot::channel();
    let control = ProxyControlApi::new(config_ref, 29381);
    tokio::spawn(async move {
        control.start_with_signal(Some(ctrl_tx)).await.unwrap();
    });
    ctrl_rx.await.unwrap();

    // Configure for openai_chat format
    let client = reqwest::Client::new();
    let _ = client
        .post("http://127.0.0.1:29381/legion/config")
        .json(&serde_json::json!({
            "target_url": "http://127.0.0.1:28380",
            "api_key": "test-key",
            "api_format": "openai_chat",
            "model": "gpt-4"
        }))
        .send()
        .await
        .unwrap();

    // Send Anthropic-format request with stream=true
    let response = client
        .post("http://127.0.0.1:28381/v1/messages")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": "claude-opus-4-6",
            "max_tokens": 1024,
            "stream": true,
            "messages": [{"role": "user", "content": "hello"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    // Response should be SSE format (wrapped Anthropic SSE)
    let ct = response.headers().get("content-type").unwrap().to_str().unwrap();
    assert!(ct.contains("text/event-stream"), "Expected SSE content-type, got: {}", ct);

    let body = response.text().await.unwrap();
    // Should contain Anthropic SSE events
    assert!(body.contains("event: message_start"), "Should have message_start event");
    assert!(body.contains("event: content_block_delta"), "Should have content_block_delta event");
    assert!(body.contains("event: message_stop"), "Should have message_stop event");
    // stream=false should be set in upstream request (the mock echoes it back)
    assert!(body.contains("stream=false"), "Should have forced stream=false: {}", body);

    // Verify URL doesn't have double /v1
    let received_url = url_rx.await.unwrap();
    assert!(
        !received_url.contains("/v1/v1"),
        "URL should not have double /v1: {}",
        received_url
    );
}
