use legion_core::orchestrate::{WorkerTaskStatus, WorkerState};

fn main() {
    // Test all variants serialize as snake_case
    let statuses = vec![
        (WorkerTaskStatus::Idle, "idle"),
        (WorkerTaskStatus::Pending, "pending"),
        (WorkerTaskStatus::Working, "working"),
        (WorkerTaskStatus::Done, "done"),
        (WorkerTaskStatus::Error, "error"),
        (WorkerTaskStatus::Stopped, "stopped"),
    ];
    
    for (status, expected) in statuses {
        let json = serde_json::to_string(&status).unwrap();
        println!("{:?} => {}", status, json);
        assert_eq!(json, format!("\"{}\"", expected), "Status {:?} should serialize as \"{}\"", status, expected);
    }
    
    // Test WorkerState serialization
    let state = WorkerState {
        worker_id: 1,
        status: WorkerTaskStatus::Working,
        ticket: Some("TICKET-123".to_string()),
        summary: Some("In progress".to_string()),
        elapsed_secs: 42,
    };
    
    let json = serde_json::to_string_pretty(&state).unwrap();
    println!("\nWorkerState:\n{}", json);
    
    println!("\nAll serialization tests passed!");
}
