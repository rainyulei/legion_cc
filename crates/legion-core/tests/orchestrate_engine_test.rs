//! Unit tests for OrchestrateEngine core state management.

use legion_core::orchestrate::{OrchestrateEngine, WorkerTaskStatus};

#[tokio::test]
async fn test_engine_init_workers() {
    let engine = OrchestrateEngine::new(3);
    let all = engine.all_status().await;
    assert_eq!(all.len(), 3);
    for ws in &all {
        assert_eq!(ws.status, WorkerTaskStatus::Idle);
        assert!(ws.ticket.is_none());
        assert!(ws.summary.is_none());
    }
    // IDs should be 1, 2, 3
    let ids: Vec<u16> = all.iter().map(|w| w.worker_id).collect();
    assert_eq!(ids, vec![1, 2, 3]);
}

#[tokio::test]
async fn test_dispatch_task() {
    let engine = OrchestrateEngine::new(3);
    engine.dispatch(1, "TICKET-42".to_string()).await.unwrap();

    let ws = engine.worker_status(1).await.unwrap();
    assert_eq!(ws.status, WorkerTaskStatus::Pending);
    assert_eq!(ws.ticket.as_deref(), Some("TICKET-42"));
}

#[tokio::test]
async fn test_dispatch_invalid_worker() {
    let engine = OrchestrateEngine::new(3);
    let result = engine.dispatch(99, "TICKET-X".to_string()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_report_done() {
    let engine = OrchestrateEngine::new(3);
    engine.dispatch(1, "TICKET-1".to_string()).await.unwrap();
    engine.mark_working(1).await;
    engine
        .report(1, WorkerTaskStatus::Done, Some("All good".to_string()))
        .await
        .unwrap();

    let ws = engine.worker_status(1).await.unwrap();
    assert_eq!(ws.status, WorkerTaskStatus::Done);
    assert_eq!(ws.summary.as_deref(), Some("All good"));
}

#[tokio::test]
async fn test_report_error() {
    let engine = OrchestrateEngine::new(3);
    engine.dispatch(1, "TICKET-ERR".to_string()).await.unwrap();
    engine.mark_working(1).await;
    engine
        .report(1, WorkerTaskStatus::Error, Some("compile failed".to_string()))
        .await
        .unwrap();

    let ws = engine.worker_status(1).await.unwrap();
    assert_eq!(ws.status, WorkerTaskStatus::Error);
    assert_eq!(ws.summary.as_deref(), Some("compile failed"));
}

#[tokio::test]
async fn test_take_pending_task() {
    let engine = OrchestrateEngine::new(3);
    engine
        .dispatch(2, "TICKET-TAKE".to_string())
        .await
        .unwrap();

    // First take should return the ticket and transition to Working
    let ticket = engine.take_pending(2).await;
    assert_eq!(ticket.as_deref(), Some("TICKET-TAKE"));

    let ws = engine.worker_status(2).await.unwrap();
    assert_eq!(ws.status, WorkerTaskStatus::Working);

    // Second take should return None (already Working)
    let ticket2 = engine.take_pending(2).await;
    assert!(ticket2.is_none());
}

#[tokio::test]
async fn test_stop_worker() {
    let engine = OrchestrateEngine::new(3);
    engine.dispatch(1, "TICKET-STOP".to_string()).await.unwrap();
    engine.mark_working(1).await;
    engine.stop_worker(1).await;

    let ws = engine.worker_status(1).await.unwrap();
    assert_eq!(ws.status, WorkerTaskStatus::Stopped);
}

#[tokio::test]
async fn test_stop_all() {
    let engine = OrchestrateEngine::new(3);

    // Dispatch and mark working all 3
    for id in 1..=3u16 {
        engine
            .dispatch(id, format!("TICKET-{}", id))
            .await
            .unwrap();
        engine.mark_working(id).await;
    }

    engine.stop_all().await;

    let all = engine.all_status().await;
    for ws in &all {
        assert_eq!(ws.status, WorkerTaskStatus::Stopped);
    }
}
