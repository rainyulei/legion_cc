use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use serde::Serialize;
use tokio::sync::RwLock;

/// Task-level status for a worker managed by the orchestration engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerTaskStatus {
    Idle,
    Pending,
    Working,
    Done,
    Error,
    Stopped,
}

/// Public snapshot of a worker's state (suitable for API responses).
#[derive(Debug, Clone, Serialize)]
pub struct WorkerState {
    pub worker_id: u16,
    pub status: WorkerTaskStatus,
    pub ticket: Option<String>,
    pub summary: Option<String>,
    pub elapsed_secs: u64,
}

/// Internal mutable state for a single worker.
#[derive(Debug, Clone)]
struct WorkerInner {
    worker_id: u16,
    status: WorkerTaskStatus,
    ticket: Option<String>,
    summary: Option<String>,
    started_at: Option<std::time::Instant>,
}

impl WorkerInner {
    fn new(worker_id: u16) -> Self {
        Self {
            worker_id,
            status: WorkerTaskStatus::Idle,
            ticket: None,
            summary: None,
            started_at: None,
        }
    }

    fn to_state(&self) -> WorkerState {
        let elapsed_secs = self
            .started_at
            .map(|t| t.elapsed().as_secs())
            .unwrap_or(0);
        WorkerState {
            worker_id: self.worker_id,
            status: self.status,
            ticket: self.ticket.clone(),
            summary: self.summary.clone(),
            elapsed_secs,
        }
    }
}

/// Thread-safe orchestration engine that tracks worker states, task queues, and results.
///
/// All public methods are async and acquire internal locks as needed.
#[derive(Clone)]
pub struct OrchestrateEngine {
    inner: Arc<RwLock<HashMap<u16, WorkerInner>>>,
}

impl OrchestrateEngine {
    /// Create a new engine with `worker_count` workers (IDs 1..=worker_count), all Idle.
    pub fn new(worker_count: u16) -> Self {
        let mut map = HashMap::new();
        for id in 1..=worker_count {
            map.insert(id, WorkerInner::new(id));
        }
        Self {
            inner: Arc::new(RwLock::new(map)),
        }
    }

    /// Assign a ticket to a worker, setting its status to Pending.
    pub async fn dispatch(&self, worker_id: u16, ticket: String) -> Result<()> {
        let mut guard = self.inner.write().await;
        let worker = guard
            .get_mut(&worker_id)
            .ok_or_else(|| anyhow::anyhow!("worker {} not found", worker_id))?;
        worker.status = WorkerTaskStatus::Pending;
        worker.ticket = Some(ticket);
        worker.summary = None;
        worker.started_at = None;
        Ok(())
    }

    /// Transition a worker from Pending to Working and record the start time.
    pub async fn mark_working(&self, worker_id: u16) {
        let mut guard = self.inner.write().await;
        if let Some(worker) = guard.get_mut(&worker_id) {
            worker.status = WorkerTaskStatus::Working;
            worker.started_at = Some(std::time::Instant::now());
        }
    }

    /// Update a worker's status and optional summary (e.g. Done or Error).
    pub async fn report(&self, worker_id: u16, status: WorkerTaskStatus, summary: Option<String>) -> Result<()> {
        let mut guard = self.inner.write().await;
        let worker = guard
            .get_mut(&worker_id)
            .ok_or_else(|| anyhow::anyhow!("worker {} not found", worker_id))?;
        worker.status = status;
        worker.summary = summary;
        Ok(())
    }

    /// If the worker is Pending, atomically transition to Working and return the ticket.
    /// Returns `None` if the worker is not in Pending state or does not exist.
    pub async fn take_pending(&self, worker_id: u16) -> Option<String> {
        let mut guard = self.inner.write().await;
        let worker = guard.get_mut(&worker_id)?;
        if worker.status != WorkerTaskStatus::Pending {
            return None;
        }
        worker.status = WorkerTaskStatus::Working;
        worker.started_at = Some(std::time::Instant::now());
        worker.ticket.clone()
    }

    /// Set a single worker to Stopped.
    pub async fn stop_worker(&self, worker_id: u16) {
        let mut guard = self.inner.write().await;
        if let Some(worker) = guard.get_mut(&worker_id) {
            worker.status = WorkerTaskStatus::Stopped;
        }
    }

    /// Stop all workers that are currently Working or Pending.
    pub async fn stop_all(&self) {
        let mut guard = self.inner.write().await;
        for worker in guard.values_mut() {
            if matches!(
                worker.status,
                WorkerTaskStatus::Working | WorkerTaskStatus::Pending
            ) {
                worker.status = WorkerTaskStatus::Stopped;
            }
        }
    }

    /// Return the state of a single worker, or `None` if the ID is unknown.
    pub async fn worker_status(&self, worker_id: u16) -> Option<WorkerState> {
        let guard = self.inner.read().await;
        guard.get(&worker_id).map(|w| w.to_state())
    }

    /// Return the state of all workers, sorted by worker_id.
    pub async fn all_status(&self) -> Vec<WorkerState> {
        let guard = self.inner.read().await;
        let mut states: Vec<WorkerState> = guard.values().map(|w| w.to_state()).collect();
        states.sort_by_key(|s| s.worker_id);
        states
    }
}
