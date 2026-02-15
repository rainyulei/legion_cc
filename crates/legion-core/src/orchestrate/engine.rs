use std::sync::Arc;

use serde::Serialize;
use tokio::sync::RwLock;

/// Status of a ticket in the shared queue
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TicketStatus {
    Queued,
    Working,
    Done,
    Error,
}

/// Team execution mode for a ticket
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamMode {
    TechLeadTeam,
    Solo,
    Custom(String),
}

impl Default for TeamMode {
    fn default() -> Self { Self::TechLeadTeam }
}

/// A ticket in the shared task queue
#[derive(Debug, Clone, Serialize)]
pub struct TaskTicket {
    pub id: usize,
    pub prompt: String,
    pub status: TicketStatus,
    pub assigned_worker: Option<u16>,
    pub team_mode: TeamMode,
    pub iteration: u16,
    pub max_iterations: u16,
    pub feedback: Option<String>,
    pub summary: Option<String>,
    #[serde(skip)]
    pub started_at: Option<std::time::Instant>,
}

impl TaskTicket {
    pub fn elapsed_secs(&self) -> u64 {
        self.started_at.map(|t| t.elapsed().as_secs()).unwrap_or(0)
    }
}

/// Snapshot of a ticket for API/UI consumption (no Instant field)
#[derive(Debug, Clone, Serialize)]
pub struct TicketSnapshot {
    pub id: usize,
    pub prompt: String,
    pub status: TicketStatus,
    pub assigned_worker: Option<u16>,
    pub team_mode: TeamMode,
    pub iteration: u16,
    pub max_iterations: u16,
    pub feedback: Option<String>,
    pub summary: Option<String>,
    pub elapsed_secs: u64,
}

struct EngineInner {
    tickets: Vec<TaskTicket>,
    next_ticket_id: usize,
    worker_count: u16,
}

/// Thread-safe orchestration engine that tracks a shared task queue.
///
/// All public methods are async and acquire internal locks as needed.
#[derive(Clone)]
pub struct OrchestrateEngine {
    inner: Arc<RwLock<EngineInner>>,
}

impl OrchestrateEngine {
    pub fn new(worker_count: u16) -> Self {
        Self {
            inner: Arc::new(RwLock::new(EngineInner {
                tickets: Vec::new(),
                next_ticket_id: 1,
                worker_count,
            })),
        }
    }

    pub async fn submit_ticket(&self, prompt: String, team_mode: TeamMode, max_iterations: u16) -> usize {
        let mut guard = self.inner.write().await;
        let id = guard.next_ticket_id;
        guard.next_ticket_id += 1;
        guard.tickets.push(TaskTicket {
            id,
            prompt,
            status: TicketStatus::Queued,
            assigned_worker: None,
            team_mode,
            iteration: 0,
            max_iterations,
            feedback: None,
            summary: None,
            started_at: None,
        });
        id
    }

    pub async fn take_next(&self, worker_id: u16) -> Option<(usize, String, TeamMode)> {
        let mut guard = self.inner.write().await;
        let already_working = guard.tickets.iter().any(|t| {
            t.assigned_worker == Some(worker_id) && t.status == TicketStatus::Working
        });
        if already_working { return None; }

        let ticket = guard.tickets.iter_mut().find(|t| t.status == TicketStatus::Queued)?;
        ticket.status = TicketStatus::Working;
        ticket.assigned_worker = Some(worker_id);
        ticket.iteration = 1;
        ticket.started_at = Some(std::time::Instant::now());
        Some((ticket.id, ticket.prompt.clone(), ticket.team_mode.clone()))
    }

    pub async fn report_iteration(
        &self, ticket_id: usize, success: bool, feedback: Option<String>,
    ) -> bool {
        let mut guard = self.inner.write().await;
        let ticket = match guard.tickets.iter_mut().find(|t| t.id == ticket_id) {
            Some(t) => t,
            None => return false,
        };

        if success {
            ticket.status = TicketStatus::Done;
            ticket.summary = feedback;
            return false;
        }

        ticket.iteration += 1;
        if ticket.iteration > ticket.max_iterations {
            ticket.status = TicketStatus::Error;
            ticket.summary = feedback;
            return false;
        }

        ticket.feedback = feedback;
        true
    }

    pub async fn worker_ticket(&self, worker_id: u16) -> Option<TicketSnapshot> {
        let guard = self.inner.read().await;
        guard.tickets.iter()
            .find(|t| t.assigned_worker == Some(worker_id) && t.status == TicketStatus::Working)
            .map(ticket_to_snapshot)
    }

    pub async fn is_worker_idle(&self, worker_id: u16) -> bool {
        let guard = self.inner.read().await;
        !guard.tickets.iter().any(|t| {
            t.assigned_worker == Some(worker_id) && t.status == TicketStatus::Working
        })
    }

    pub async fn all_tickets(&self) -> Vec<TicketSnapshot> {
        let guard = self.inner.read().await;
        guard.tickets.iter().map(ticket_to_snapshot).collect()
    }

    pub async fn queue_stats(&self) -> (usize, usize, usize, usize, usize) {
        let guard = self.inner.read().await;
        let total = guard.tickets.len();
        let queued = guard.tickets.iter().filter(|t| t.status == TicketStatus::Queued).count();
        let working = guard.tickets.iter().filter(|t| t.status == TicketStatus::Working).count();
        let done = guard.tickets.iter().filter(|t| t.status == TicketStatus::Done).count();
        let error = guard.tickets.iter().filter(|t| t.status == TicketStatus::Error).count();
        (total, queued, working, done, error)
    }

    pub async fn worker_count(&self) -> u16 {
        self.inner.read().await.worker_count
    }

    pub async fn set_worker_count(&self, count: u16) {
        self.inner.write().await.worker_count = count;
    }

    pub async fn stop_all(&self) {
        let mut guard = self.inner.write().await;
        for ticket in guard.tickets.iter_mut() {
            if ticket.status == TicketStatus::Working {
                ticket.status = TicketStatus::Error;
                ticket.summary = Some("Stopped by user".into());
            }
        }
    }
}

fn ticket_to_snapshot(t: &TaskTicket) -> TicketSnapshot {
    TicketSnapshot {
        id: t.id,
        prompt: t.prompt.clone(),
        status: t.status,
        assigned_worker: t.assigned_worker,
        team_mode: t.team_mode.clone(),
        iteration: t.iteration,
        max_iterations: t.max_iterations,
        feedback: t.feedback.clone(),
        summary: t.summary.clone(),
        elapsed_secs: t.elapsed_secs(),
    }
}
