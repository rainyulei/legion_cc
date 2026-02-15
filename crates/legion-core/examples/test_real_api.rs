//! Quick real API test - run with: cargo run --example test_real_api
//! Tests proxy against actual OpenCode Zen API

use legion_core::proxy::{ProxyControlApi, ProxyServer};

#[tokio::main]
async fn main() {
    // Start proxy
    let proxy = ProxyServer::new(38080);
    let config_ref = proxy.config_ref();
    let (proxy_tx, proxy_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        proxy.start_with_signal(Some(proxy_tx)).await.unwrap();
    });
    proxy_rx.await.unwrap();
    println!("✓ Proxy started on :38080");

    // Start control API
    let (ctrl_tx, ctrl_rx) = tokio::sync::oneshot::channel();
    let control = ProxyControlApi::new(config_ref, 39080);
    tokio::spawn(async move {
        control.start_with_signal(Some(ctrl_tx)).await.unwrap();
    });
    ctrl_rx.await.unwrap();
    println!("✓ Control API started on :39080");

    // Configure proxy with real OpenCode Zen
    let client = reqwest::Client::new();
    let resp = client
        .post("http://127.0.0.1:39080/legion/config")
        .json(&serde_json::json!({
            "target_url": "https://opencode.ai/zen/v1",
            "api_key": "public",
            "api_format": "openai_chat",
            "model": "kimi-k2.5-free"
        }))
        .send()
        .await
        .unwrap();
    println!("✓ Config set: {}", resp.text().await.unwrap());

    // Verify config
    let status = client
        .get("http://127.0.0.1:39080/legion/status")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    println!("✓ Status: {}", status);

    // Send real request through proxy (as Claude Code would)
    println!("\n--- Sending request to proxy (stream=true) ---");
    let response = client
        .post("http://127.0.0.1:38080/v1/messages")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": "claude-opus-4-6",
            "max_tokens": 256,
            "stream": true,
            "messages": [{"role": "user", "content": "Say just 'hello' in one word."}]
        }))
        .send()
        .await;

    match response {
        Ok(resp) => {
            let status = resp.status();
            let headers = resp.headers().clone();
            let ct = headers
                .get("content-type")
                .map(|v| v.to_str().unwrap_or("?"))
                .unwrap_or("missing");
            println!("Status: {}", status);
            println!("Content-Type: {}", ct);

            let body = resp.text().await.unwrap_or_else(|e| format!("ERROR: {}", e));
            // Show first 1000 chars
            let preview = if body.len() > 1000 {
                format!("{}...(truncated, total {} bytes)", &body[..1000], body.len())
            } else {
                body.clone()
            };
            println!("Body:\n{}", preview);

            if status.is_success() && ct.contains("text/event-stream") {
                println!("\n✅ SUCCESS: Got SSE streaming response!");
            } else if status.is_success() {
                println!("\n✅ SUCCESS: Got response (non-SSE)");
            } else {
                println!("\n❌ FAILED: Status {}", status);
            }
        }
        Err(e) => {
            println!("❌ REQUEST FAILED: {}", e);
        }
    }
}
