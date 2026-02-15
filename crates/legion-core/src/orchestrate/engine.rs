use std::sync::{Arc, Mutex};

use legion_db::{Repository, TicketRow};
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
#[allow(dead_code)]
pub struct TaskTicket {
    pub id: usize,
    pub prompt: String,
    pub title: String,
    pub context: Option<String>,
    pub criteria: Option<String>,
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
    pub title: String,
    pub context: Option<String>,
    pub criteria: Option<String>,
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
    db: Option<Arc<Mutex<Repository>>>,
    session_name: Option<String>,
}

impl OrchestrateEngine {
    pub fn new(worker_count: u16) -> Self {
        Self {
            inner: Arc::new(RwLock::new(EngineInner {
                tickets: Vec::new(),
                next_ticket_id: 1,
                worker_count,
            })),
            db: None,
            session_name: None,
        }
    }

    pub fn with_db(worker_count: u16, repo: Arc<Mutex<Repository>>, session_name: String) -> Self {
        // Load existing tickets from DB
        let mut tickets = Vec::new();
        let mut next_id = 1usize;
        if let Ok(db) = repo.lock() {
            if let Ok(rows) = db.list_tickets_by_session(&session_name) {
                for row in rows {
                    let status = match row.status.as_str() {
                        "queued" => TicketStatus::Queued,
                        "working" => TicketStatus::Queued, // Reset working→queued on restart
                        "done" => TicketStatus::Done,
                        "error" => TicketStatus::Error,
                        _ => TicketStatus::Queued,
                    };
                    let team_mode = match row.team_mode.as_str() {
                        "tech_lead_team" => TeamMode::TechLeadTeam,
                        "solo" => TeamMode::Solo,
                        other => TeamMode::Custom(other.to_string()),
                    };
                    tickets.push(TaskTicket {
                        id: row.id as usize,
                        prompt: row.prompt,
                        title: row.title,
                        context: row.context,
                        criteria: row.criteria,
                        status,
                        assigned_worker: row.assigned_worker.map(|w| w as u16),
                        team_mode,
                        iteration: row.iteration as u16,
                        max_iterations: row.max_iterations as u16,
                        feedback: row.feedback,
                        summary: row.summary,
                        started_at: None,
                    });
                    if row.id as usize >= next_id {
                        next_id = row.id as usize + 1;
                    }
                }
            }
        }
        // Reset assigned_worker for queued tickets (were working before restart)
        for t in tickets.iter_mut() {
            if t.status == TicketStatus::Queued {
                t.assigned_worker = None;
            }
        }
        Self {
            inner: Arc::new(RwLock::new(EngineInner {
                tickets,
                next_ticket_id: next_id,
                worker_count,
            })),
            db: Some(repo),
            session_name: Some(session_name),
        }
    }

    fn persist_ticket(&self, ticket: &TaskTicket) {
        if let (Some(db), Some(session)) = (&self.db, &self.session_name) {
            let now = chrono::Utc::now().timestamp();
            let team_str = match &ticket.team_mode {
                TeamMode::TechLeadTeam => "tech_lead_team".to_string(),
                TeamMode::Solo => "solo".to_string(),
                TeamMode::Custom(s) => s.clone(),
            };
            let status_str = match ticket.status {
                TicketStatus::Queued => "queued",
                TicketStatus::Working => "working",
                TicketStatus::Done => "done",
                TicketStatus::Error => "error",
            };
            let row = TicketRow {
                id: ticket.id as i64,
                session_name: session.clone(),
                title: ticket.title.clone(),
                prompt: ticket.prompt.clone(),
                context: ticket.context.clone(),
                criteria: ticket.criteria.clone(),
                status: status_str.to_string(),
                assigned_worker: ticket.assigned_worker.map(|w| w as i64),
                team_mode: team_str,
                iteration: ticket.iteration as i64,
                max_iterations: ticket.max_iterations as i64,
                feedback: ticket.feedback.clone(),
                summary: ticket.summary.clone(),
                created_at: now,
                updated_at: now,
            };
            if let Ok(db) = db.lock() {
                let _ = db.insert_ticket(&row);
            }
        }
    }

    fn persist_ticket_update(&self, ticket: &TaskTicket) {
        if let (Some(db), Some(session)) = (&self.db, &self.session_name) {
            let now = chrono::Utc::now().timestamp();
            let status_str = match ticket.status {
                TicketStatus::Queued => "queued",
                TicketStatus::Working => "working",
                TicketStatus::Done => "done",
                TicketStatus::Error => "error",
            };
            let team_str = match &ticket.team_mode {
                TeamMode::TechLeadTeam => "tech_lead_team".to_string(),
                TeamMode::Solo => "solo".to_string(),
                TeamMode::Custom(s) => s.clone(),
            };
            let row = TicketRow {
                id: ticket.id as i64,
                session_name: session.clone(),
                title: ticket.title.clone(),
                prompt: ticket.prompt.clone(),
                context: ticket.context.clone(),
                criteria: ticket.criteria.clone(),
                status: status_str.to_string(),
                assigned_worker: ticket.assigned_worker.map(|w| w as i64),
                team_mode: team_str,
                iteration: ticket.iteration as i64,
                max_iterations: ticket.max_iterations as i64,
                feedback: ticket.feedback.clone(),
                summary: ticket.summary.clone(),
                created_at: 0, // not updated
                updated_at: now,
            };
            if let Ok(db) = db.lock() {
                let _ = db.update_ticket(&row);
            }
        }
    }

    pub async fn submit_ticket(
        &self, title: String, prompt: String, context: Option<String>, criteria: Option<String>,
        team_mode: TeamMode, max_iterations: u16,
    ) -> usize {
        let mut guard = self.inner.write().await;
        let id = guard.next_ticket_id;
        guard.next_ticket_id += 1;
        let ticket = TaskTicket {
            id,
            prompt,
            title,
            context,
            criteria,
            status: TicketStatus::Queued,
            assigned_worker: None,
            team_mode,
            iteration: 0,
            max_iterations,
            feedback: None,
            summary: None,
            started_at: None,
        };
        guard.tickets.push(ticket.clone());
        drop(guard);
        self.persist_ticket(&ticket);
        id
    }

    pub async fn take_next(&self, worker_id: u16) -> Option<TicketSnapshot> {
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
        let snap = ticket_to_snapshot(ticket);
        let persisted = ticket.clone();
        drop(guard);
        self.persist_ticket_update(&persisted);
        Some(snap)
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
            let snap = ticket.clone();
            drop(guard);
            self.persist_ticket_update(&snap);
            return false;
        }

        if ticket.iteration >= ticket.max_iterations {
            ticket.status = TicketStatus::Error;
            ticket.summary = feedback;
            let snap = ticket.clone();
            drop(guard);
            self.persist_ticket_update(&snap);
            return false;
        }

        ticket.iteration += 1;
        ticket.feedback = feedback;
        let snap = ticket.clone();
        drop(guard);
        self.persist_ticket_update(&snap);
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

    pub fn db(&self) -> Option<&Arc<Mutex<Repository>>> {
        self.db.as_ref()
    }

    pub fn session_name(&self) -> Option<&str> {
        self.session_name.as_deref()
    }

    /// Retry a failed ticket: reset Error → Queued for re-execution
    pub async fn retry_ticket(&self, ticket_id: usize) -> bool {
        let mut guard = self.inner.write().await;
        let ticket = match guard.tickets.iter_mut().find(|t| t.id == ticket_id) {
            Some(t) => t,
            None => return false,
        };
        if ticket.status != TicketStatus::Error {
            return false;
        }
        ticket.status = TicketStatus::Queued;
        ticket.assigned_worker = None;
        ticket.iteration = 0;
        ticket.feedback = None;
        ticket.summary = None;
        ticket.started_at = None;
        let snap = ticket.clone();
        drop(guard);
        self.persist_ticket_update(&snap);
        true
    }

    /// Delete a single completed/errored ticket
    pub async fn delete_ticket(&self, ticket_id: usize) -> bool {
        let mut guard = self.inner.write().await;
        let idx = guard.tickets.iter().position(|t| t.id == ticket_id);
        if let Some(i) = idx {
            let status = guard.tickets[i].status;
            if matches!(status, TicketStatus::Done | TicketStatus::Error) {
                guard.tickets.remove(i);
                drop(guard);
                self.delete_ticket_from_db(ticket_id);
                return true;
            }
        }
        false
    }

    /// Clear all completed (Done + Error) tickets
    pub async fn clear_completed(&self) -> Vec<usize> {
        let mut guard = self.inner.write().await;
        let mut removed_ids = Vec::new();
        guard.tickets.retain(|t| {
            if matches!(t.status, TicketStatus::Done | TicketStatus::Error) {
                removed_ids.push(t.id);
                false
            } else {
                true
            }
        });
        drop(guard);
        for id in &removed_ids {
            self.delete_ticket_from_db(*id);
        }
        removed_ids
    }

    fn delete_ticket_from_db(&self, ticket_id: usize) {
        if let (Some(db), Some(session)) = (&self.db, &self.session_name) {
            if let Ok(db) = db.lock() {
                let _ = db.delete_ticket(ticket_id as i64, session);
            }
        }
    }

    pub async fn stop_all(&self) {
        let mut guard = self.inner.write().await;
        let mut stopped = Vec::new();
        for ticket in guard.tickets.iter_mut() {
            if ticket.status == TicketStatus::Working {
                ticket.status = TicketStatus::Error;
                ticket.summary = Some("Stopped by user".into());
                stopped.push(ticket.clone());
            }
        }
        drop(guard);
        for t in &stopped {
            self.persist_ticket_update(t);
        }
    }
}

fn ticket_to_snapshot(t: &TaskTicket) -> TicketSnapshot {
    TicketSnapshot {
        id: t.id,
        prompt: t.prompt.clone(),
        title: t.title.clone(),
        context: t.context.clone(),
        criteria: t.criteria.clone(),
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
