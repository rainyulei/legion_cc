use legion_core::{OrchestrateEngine, OrchestrateApi, WorkerTaskStatus};
use tokio::sync::oneshot;

/// Helper: create an engine + API on the given port, start in background, wait until ready.
async fn start_api(worker_count: u16, port: u16) -> OrchestrateEngine {
    let engine = OrchestrateEngine::new(worker_count);
    let api = OrchestrateApi::new(engine.clone(), port);

    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        api.start_with_signal(Some(tx)).await.unwrap();
    });
    rx.await.expect("API ready signal");
    engine
}

fn base_url(port: u16) -> String {
    format!("http://127.0.0.1:{}", port)
}

#[tokio::test]
async fn test_status_endpoint() {
    let _engine = start_api(2, 30080).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/legion/orchestrate/status", base_url(30080)))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let workers = body["workers"].as_array().unwrap();
    assert_eq!(workers.len(), 2);

    // Both should be idle
    for w in workers {
        assert_eq!(w["status"].as_str().unwrap(), "idle");
    }
    // Sorted by worker_id
    assert_eq!(workers[0]["worker_id"].as_u64().unwrap(), 1);
    assert_eq!(workers[1]["worker_id"].as_u64().unwrap(), 2);
}

#[tokio::test]
async fn test_dispatch_endpoint() {
    let engine = start_api(2, 30081).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/legion/orchestrate/dispatch", base_url(30081)))
        .json(&serde_json::json!({
            "worker_id": 1,
            "ticket": "fix-bug-123"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"].as_str().unwrap(), "dispatched");

    // Verify engine state changed
    let state = engine.worker_status(1).await.unwrap();
    assert_eq!(state.status, WorkerTaskStatus::Pending);
    assert_eq!(state.ticket.as_deref(), Some("fix-bug-123"));
}

#[tokio::test]
async fn test_report_endpoint() {
    let engine = start_api(2, 30082).await;

    // Setup: dispatch and mark working first
    engine.dispatch(1, "task-abc".to_string()).await.unwrap();
    engine.mark_working(1).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/legion/orchestrate/report", base_url(30082)))
        .json(&serde_json::json!({
            "worker_id": 1,
            "status": "done",
            "summary": "All tests passed"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"].as_str().unwrap(), "ok");

    // Verify engine state
    let state = engine.worker_status(1).await.unwrap();
    assert_eq!(state.status, WorkerTaskStatus::Done);
    assert_eq!(state.summary.as_deref(), Some("All tests passed"));
}

#[tokio::test]
async fn test_stop_endpoint() {
    let engine = start_api(2, 30083).await;

    // Setup: dispatch and mark working
    engine.dispatch(1, "task-xyz".to_string()).await.unwrap();
    engine.mark_working(1).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/legion/orchestrate/stop", base_url(30083)))
        .json(&serde_json::json!({
            "worker_id": 1
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"].as_str().unwrap(), "stopped");

    // Verify engine state
    let state = engine.worker_status(1).await.unwrap();
    assert_eq!(state.status, WorkerTaskStatus::Stopped);
}

#[tokio::test]
async fn test_stop_all_endpoint() {
    let engine = start_api(3, 30084).await;

    // Setup: dispatch and mark all working
    for id in 1..=3u16 {
        engine
            .dispatch(id, format!("task-{}", id))
            .await
            .unwrap();
        engine.mark_working(id).await;
    }

    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/legion/orchestrate/stop-all",
            base_url(30084)
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"].as_str().unwrap(), "all_stopped");

    // Verify all workers stopped
    let states = engine.all_status().await;
    for state in &states {
        assert_eq!(state.status, WorkerTaskStatus::Stopped);
    }
}

#[tokio::test]
async fn test_404() {
    let _engine = start_api(1, 30085).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "{}/legion/orchestrate/nonexistent",
            base_url(30085)
        ))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("not found"));
}
