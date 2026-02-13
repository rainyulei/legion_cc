//! Integration test for proxy + control API

use legion_core::proxy::{ProxyControlApi, ProxyServer};

#[tokio::test]
async fn test_control_api_status() {
    let proxy = ProxyServer::new(28080);
    let config_ref = proxy.config_ref();

    // Start proxy
    let (proxy_tx, proxy_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        proxy.start_with_signal(Some(proxy_tx)).await.unwrap();
    });
    proxy_rx.await.unwrap();

    // Start control API
    let (ctrl_tx, ctrl_rx) = tokio::sync::oneshot::channel();
    let control = ProxyControlApi::new(config_ref, 29080);
    tokio::spawn(async move {
        control.start_with_signal(Some(ctrl_tx)).await.unwrap();
    });
    ctrl_rx.await.unwrap();

    // Test GET /legion/status
    let client = reqwest::Client::new();
    let resp = client
        .get("http://127.0.0.1:29080/legion/status")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["configured"], false);
    assert!(body["target_url"].is_null());

    // Test POST /legion/config
    let resp = client
        .post("http://127.0.0.1:29080/legion/config")
        .json(&serde_json::json!({
            "target_url": "https://api.example.com/v1",
            "api_format": "openai_chat",
            "model": "gpt-4"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Verify status reflects the update
    let resp = client
        .get("http://127.0.0.1:29080/legion/status")
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["target_url"], "https://api.example.com/v1");
    assert_eq!(body["api_format"], "openai_chat");
    assert_eq!(body["model"], "gpt-4");
    // Still not "configured" because api_key is missing
    assert_eq!(body["configured"], false);

    // Add api_key
    let resp = client
        .post("http://127.0.0.1:29080/legion/config")
        .json(&serde_json::json!({
            "api_key": "sk-test-key"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // Now should be configured
    let resp = client
        .get("http://127.0.0.1:29080/legion/status")
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["configured"], true);

    // Test GET /legion/providers
    let resp = client
        .get("http://127.0.0.1:29080/legion/providers")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.is_array());

    // Test 404
    let resp = client
        .get("http://127.0.0.1:29080/legion/nonexistent")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn test_control_api_bad_json() {
    let proxy = ProxyServer::new(28081);
    let config_ref = proxy.config_ref();

    let (proxy_tx, proxy_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        proxy.start_with_signal(Some(proxy_tx)).await.unwrap();
    });
    proxy_rx.await.unwrap();

    let (ctrl_tx, ctrl_rx) = tokio::sync::oneshot::channel();
    let control = ProxyControlApi::new(config_ref, 29081);
    tokio::spawn(async move {
        control.start_with_signal(Some(ctrl_tx)).await.unwrap();
    });
    ctrl_rx.await.unwrap();

    let client = reqwest::Client::new();
    let resp = client
        .post("http://127.0.0.1:29081/legion/config")
        .body("not json at all")
        .header("content-type", "application/json")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}
