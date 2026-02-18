use legion_core::{OrchestrateEngine, OrchestrateApi, TicketStatus};
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
async fn test_status_endpoint_empty() {
    let _engine = start_api(2, 30080).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/legion/orchestrate/status", base_url(30080)))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let tickets = body["tickets"].as_array().unwrap();
    assert!(tickets.is_empty());
    assert_eq!(body["total"].as_u64().unwrap(), 0);
}

#[tokio::test]
async fn test_submit_endpoint() {
    let engine = start_api(2, 30081).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/legion/orchestrate/submit", base_url(30081)))
        .json(&serde_json::json!({
            "title": "Fix bug 123",
            "ticket": "fix-bug-123",
            "team_mode": "tech_lead_team"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ticket_id"].as_u64().unwrap(), 1);

    // Verify engine state
    let all = engine.all_tickets().await;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].status, TicketStatus::Queued);
    assert_eq!(all[0].prompt, "fix-bug-123");
}

#[tokio::test]
async fn test_dispatch_compat_endpoint() {
    let engine = start_api(2, 30086).await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/legion/orchestrate/dispatch", base_url(30086)))
        .json(&serde_json::json!({
            "worker_id": 1,
            "ticket": "compat-task"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"].as_str().unwrap(), "dispatched");
    assert!(body["ticket_id"].as_u64().is_some());

    // Verify ticket was queued
    let all = engine.all_tickets().await;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].prompt, "compat-task");
}

#[tokio::test]
async fn test_report_endpoint() {
    let engine = start_api(2, 30082).await;

    // Setup: submit ticket and take it
    engine.submit_ticket("task-abc".into(), "task-abc".into(), None, None, legion_core::TeamMode::default(), 5, Vec::new()).await.unwrap();
    engine.take_next(1).await;

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
    let all = engine.all_tickets().await;
    assert_eq!(all[0].status, TicketStatus::Done);
    assert_eq!(all[0].summary.as_deref(), Some("All tests passed"));
}

#[tokio::test]
async fn test_stop_all_endpoint() {
    let engine = start_api(3, 30084).await;

    // Setup: submit and take tickets
    for i in 1..=3u16 {
        engine.submit_ticket(format!("task-{}", i), format!("task-{}", i), None, None, legion_core::TeamMode::default(), 5, Vec::new()).await.unwrap();
    }
    for i in 1..=3u16 {
        engine.take_next(i).await;
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

    // Verify all tickets errored (stopped)
    let all = engine.all_tickets().await;
    for t in &all {
        assert_eq!(t.status, TicketStatus::Error);
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
