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

/// Spawn a proxy + control API pair, return the control port
async fn spawn_pane(proxy_port: u16, control_port: u16) {
    let proxy = ProxyServer::new(proxy_port);
    let config_ref = proxy.config_ref();

    let (ptx, prx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        proxy.start_with_signal(Some(ptx)).await.unwrap();
    });
    prx.await.unwrap();

    let (ctx, crx) = tokio::sync::oneshot::channel();
    let control = ProxyControlApi::new(config_ref, control_port);
    tokio::spawn(async move {
        control.start_with_signal(Some(ctx)).await.unwrap();
    });
    crx.await.unwrap();
}

/// Helper: GET /legion/status and return parsed JSON
async fn get_status(client: &reqwest::Client, control_port: u16) -> serde_json::Value {
    client
        .get(format!("http://127.0.0.1:{}/legion/status", control_port))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// Helper: POST /legion/config
async fn set_config(client: &reqwest::Client, control_port: u16, body: &serde_json::Value) {
    let resp = client
        .post(format!("http://127.0.0.1:{}/legion/config", control_port))
        .json(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

/// Test: squad mode proxy isolation — config changes to one pane
/// do NOT leak to other panes.
#[tokio::test]
async fn test_squad_proxy_isolation() {
    // Simulate squad with 3 panes: leader + 2 workers
    let base_port: u16 = 28200;
    let ports: Vec<(u16, u16)> = vec![
        (base_port, base_port + 1000),         // Leader
        (base_port + 1, base_port + 1000 + 1), // Worker 1
        (base_port + 2, base_port + 1000 + 2), // Worker 2
    ];

    for &(pp, cp) in &ports {
        spawn_pane(pp, cp).await;
    }

    let client = reqwest::Client::new();

    // 1. All panes start unconfigured
    for &(_, cp) in &ports {
        let status = get_status(&client, cp).await;
        assert_eq!(status["configured"], false, "pane :{} should start unconfigured", cp);
        assert!(status["target_url"].is_null());
    }

    // 2. Configure Leader → Anthropic provider
    set_config(&client, ports[0].1, &serde_json::json!({
        "target_url": "https://api.anthropic.com",
        "api_format": "anthropic",
        "api_key": "sk-leader-key",
        "model": "claude-opus-4-6"
    })).await;

    // 3. Configure Worker 1 → OpenCode Zen
    set_config(&client, ports[1].1, &serde_json::json!({
        "target_url": "https://opencode.ai/zen/v1",
        "api_format": "anthropic_bearer",
        "api_key": "free",
        "model": "glm-4.7-free"
    })).await;

    // 4. Configure Worker 2 → OpenAI
    set_config(&client, ports[2].1, &serde_json::json!({
        "target_url": "https://api.openai.com/v1",
        "api_format": "openai_chat",
        "api_key": "sk-openai-key",
        "model": "gpt-5.2"
    })).await;

    // 5. Verify ISOLATION: each pane has its own config

    let leader = get_status(&client, ports[0].1).await;
    assert_eq!(leader["target_url"], "https://api.anthropic.com");
    assert_eq!(leader["api_format"], "anthropic");
    assert_eq!(leader["model"], "claude-opus-4-6");
    assert_eq!(leader["configured"], true);

    let worker1 = get_status(&client, ports[1].1).await;
    assert_eq!(worker1["target_url"], "https://opencode.ai/zen/v1");
    assert_eq!(worker1["api_format"], "anthropic_bearer");
    assert_eq!(worker1["model"], "glm-4.7-free");
    assert_eq!(worker1["configured"], true);

    let worker2 = get_status(&client, ports[2].1).await;
    assert_eq!(worker2["target_url"], "https://api.openai.com/v1");
    assert_eq!(worker2["api_format"], "openai_chat");
    assert_eq!(worker2["model"], "gpt-5.2");
    assert_eq!(worker2["configured"], true);

    // 6. Update Worker 1 only — verify Leader and Worker 2 unchanged
    set_config(&client, ports[1].1, &serde_json::json!({
        "target_url": "https://api.githubcopilot.com",
        "api_format": "anthropic_bearer",
        "api_key": "gho-test-token",
        "model": "claude-sonnet-4"
    })).await;

    // Worker 1 changed
    let worker1 = get_status(&client, ports[1].1).await;
    assert_eq!(worker1["target_url"], "https://api.githubcopilot.com");
    assert_eq!(worker1["model"], "claude-sonnet-4");

    // Leader NOT affected
    let leader = get_status(&client, ports[0].1).await;
    assert_eq!(leader["target_url"], "https://api.anthropic.com");
    assert_eq!(leader["model"], "claude-opus-4-6");

    // Worker 2 NOT affected
    let worker2 = get_status(&client, ports[2].1).await;
    assert_eq!(worker2["target_url"], "https://api.openai.com/v1");
    assert_eq!(worker2["model"], "gpt-5.2");
}

/// Test: auth headers differ per api_format (anthropic vs anthropic_bearer vs openai_chat).
/// We can't easily verify actual upstream headers without a mock server,
/// so we verify config persistence is correct per format.
#[tokio::test]
async fn test_api_format_config_persistence() {
    let client = reqwest::Client::new();

    // Pane A: anthropic format
    spawn_pane(28210, 29210).await;
    set_config(&client, 29210, &serde_json::json!({
        "target_url": "https://api.anthropic.com",
        "api_format": "anthropic",
        "api_key": "sk-ant-key"
    })).await;

    // Pane B: anthropic_bearer format
    spawn_pane(28211, 29211).await;
    set_config(&client, 29211, &serde_json::json!({
        "target_url": "https://api.githubcopilot.com",
        "api_format": "anthropic_bearer",
        "api_key": "gho-token"
    })).await;

    // Pane C: openai_chat format
    spawn_pane(28212, 29212).await;
    set_config(&client, 29212, &serde_json::json!({
        "target_url": "https://api.openai.com/v1",
        "api_format": "openai_chat",
        "api_key": "sk-openai-key"
    })).await;

    let a = get_status(&client, 29210).await;
    let b = get_status(&client, 29211).await;
    let c = get_status(&client, 29212).await;

    assert_eq!(a["api_format"], "anthropic");
    assert_eq!(b["api_format"], "anthropic_bearer");
    assert_eq!(c["api_format"], "openai_chat");

    // All configured
    assert_eq!(a["configured"], true);
    assert_eq!(b["configured"], true);
    assert_eq!(c["configured"], true);
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
