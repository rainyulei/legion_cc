//! Unit tests for OrchestrateEngine shared task queue.

use legion_core::orchestrate::{OrchestrateEngine, TicketStatus, TeamMode};

#[tokio::test]
async fn test_engine_init_empty() {
    let engine = OrchestrateEngine::new(3);
    let all = engine.all_tickets().await;
    assert!(all.is_empty());
    assert_eq!(engine.worker_count().await, 3);

    let (total, queued, working, done, error) = engine.queue_stats().await;
    assert_eq!((total, queued, working, done, error), (0, 0, 0, 0, 0));
}

#[tokio::test]
async fn test_submit_ticket() {
    let engine = OrchestrateEngine::new(2);
    let id = engine.submit_ticket("Fix bug #42".into(), "Fix bug #42".into(), None, None, TeamMode::TechLeadTeam, 5, Vec::new(), None).await.unwrap();
    assert_eq!(id, 1);

    let all = engine.all_tickets().await;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].status, TicketStatus::Queued);
    assert_eq!(all[0].prompt, "Fix bug #42");
    assert!(all[0].assigned_worker.is_none());
}

#[tokio::test]
async fn test_take_next() {
    let engine = OrchestrateEngine::new(2);
    engine.submit_ticket("Task A".into(), "Task A".into(), None, None, TeamMode::Solo, 3, Vec::new(), None).await.unwrap();

    // Worker 1 takes the ticket
    let taken = engine.take_next(1).await;
    assert!(taken.is_some());
    let snap = taken.unwrap();
    assert_eq!(snap.id, 1);
    assert_eq!(snap.prompt, "Task A");

    // Worker 1 should not take another (already working)
    let taken2 = engine.take_next(1).await;
    assert!(taken2.is_none());

    // No more queued tickets
    let taken3 = engine.take_next(2).await;
    assert!(taken3.is_none());
}

#[tokio::test]
async fn test_report_iteration_success() {
    let engine = OrchestrateEngine::new(2);
    engine.submit_ticket("Task B".into(), "Task B".into(), None, None, TeamMode::TechLeadTeam, 5, Vec::new(), None).await.unwrap();
    engine.take_next(1).await;

    let should_retry = engine.report_iteration(1, true, Some("All good".into())).await;
    assert!(!should_retry);

    let all = engine.all_tickets().await;
    assert_eq!(all[0].status, TicketStatus::Done);
    assert_eq!(all[0].summary.as_deref(), Some("All good"));
}

#[tokio::test]
async fn test_report_iteration_retry() {
    let engine = OrchestrateEngine::new(2);
    engine.submit_ticket("Task C".into(), "Task C".into(), None, None, TeamMode::TechLeadTeam, 3, Vec::new(), None).await.unwrap();
    engine.take_next(1).await;

    // First failure — should retry (iteration 2 <= max 3)
    let should_retry = engine.report_iteration(1, false, Some("Tests failed".into())).await;
    assert!(should_retry);

    let ticket = engine.worker_ticket(1).await.unwrap();
    assert_eq!(ticket.status, TicketStatus::Working);
    assert_eq!(ticket.iteration, 2);
    assert_eq!(ticket.feedback.as_deref(), Some("Tests failed"));
}

#[tokio::test]
async fn test_report_iteration_max_exceeded() {
    let engine = OrchestrateEngine::new(2);
    engine.submit_ticket("Task D".into(), "Task D".into(), None, None, TeamMode::Solo, 2, Vec::new(), None).await.unwrap();
    engine.take_next(1).await;

    // Fail iteration 1 -> retry (iter becomes 2)
    let r1 = engine.report_iteration(1, false, Some("fail 1".into())).await;
    assert!(r1); // should retry

    // Fail iteration 2 -> iteration becomes 3 > max_iterations 2 -> Error
    let r2 = engine.report_iteration(1, false, Some("fail 2".into())).await;
    assert!(!r2); // no more retries

    let all = engine.all_tickets().await;
    assert_eq!(all[0].status, TicketStatus::Error);
}

#[tokio::test]
async fn test_stop_all() {
    let engine = OrchestrateEngine::new(3);
    engine.submit_ticket("T1".into(), "T1".into(), None, None, TeamMode::default(), 5, Vec::new(), None).await.unwrap();
    engine.submit_ticket("T2".into(), "T2".into(), None, None, TeamMode::default(), 5, Vec::new(), None).await.unwrap();
    engine.take_next(1).await;
    engine.take_next(2).await;

    engine.stop_all().await;

    let all = engine.all_tickets().await;
    for t in &all {
        assert_eq!(t.status, TicketStatus::Error);
        assert_eq!(t.summary.as_deref(), Some("Stopped by user"));
    }
}

#[tokio::test]
async fn test_queue_stats() {
    let engine = OrchestrateEngine::new(2);
    engine.submit_ticket("T1".into(), "T1".into(), None, None, TeamMode::default(), 5, Vec::new(), None).await.unwrap();
    engine.submit_ticket("T2".into(), "T2".into(), None, None, TeamMode::default(), 5, Vec::new(), None).await.unwrap();
    engine.submit_ticket("T3".into(), "T3".into(), None, None, TeamMode::default(), 5, Vec::new(), None).await.unwrap();
    engine.take_next(1).await; // T1 -> Working
    engine.report_iteration(1, true, None).await; // T1 -> Done
    engine.take_next(1).await; // T2 -> Working

    let (total, queued, working, done, error) = engine.queue_stats().await;
    assert_eq!(total, 3);
    assert_eq!(queued, 1);  // T3
    assert_eq!(working, 1); // T2
    assert_eq!(done, 1);    // T1
    assert_eq!(error, 0);
}

#[tokio::test]
async fn test_worker_idle() {
    let engine = OrchestrateEngine::new(2);
    assert!(engine.is_worker_idle(1).await);

    engine.submit_ticket("Task".into(), "Task".into(), None, None, TeamMode::default(), 5, Vec::new(), None).await.unwrap();
    engine.take_next(1).await;
    assert!(!engine.is_worker_idle(1).await);
    assert!(engine.is_worker_idle(2).await);
}
