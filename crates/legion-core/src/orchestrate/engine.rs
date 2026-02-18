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

/// Merge status of a completed ticket's code into the leader branch
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStatus {
    Pending,
    Merged,
    Conflict,
    Skipped,
}

impl Default for MergeStatus {
    fn default() -> Self { Self::Pending }
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
    /// Frozen elapsed time for Done/Error tickets (so it stops counting)
    pub completed_elapsed_secs: Option<u64>,
    /// Git commit SHA when worker started this ticket (for diff base)
    pub base_commit: Option<String>,
    pub blocked_by: Vec<usize>,
    pub merge_status: MergeStatus,
}

impl TaskTicket {
    pub fn elapsed_secs(&self) -> u64 {
        if let Some(frozen) = self.completed_elapsed_secs {
            return frozen;
        }
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
    pub base_commit: Option<String>,
    pub blocked_by: Vec<usize>,
    pub merge_status: MergeStatus,
}

struct EngineInner {
    tickets: Vec<TaskTicket>,
    next_ticket_id: usize,
    worker_count: u16,
    default_max_iterations: u16,
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
                default_max_iterations: 5,
            })),
            db: None,
            session_name: None,
        }
    }

    pub async fn set_default_max_iterations(&self, n: u16) {
        self.inner.write().await.default_max_iterations = n;
    }

    pub async fn default_max_iterations(&self) -> u16 {
        self.inner.read().await.default_max_iterations
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
                        completed_elapsed_secs: None,
                        base_commit: row.base_commit.clone(),
                        blocked_by: serde_json::from_str(&row.blocked_by).unwrap_or_default(),
                        merge_status: match row.merge_status.as_str() {
                            "merged" => MergeStatus::Merged,
                            "conflict" => MergeStatus::Conflict,
                            "skipped" => MergeStatus::Skipped,
                            _ => MergeStatus::Pending,
                        },
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
                default_max_iterations: 5,
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
                origin_session: None,
                base_commit: ticket.base_commit.clone(),
                blocked_by: serde_json::to_string(&ticket.blocked_by).unwrap_or_else(|_| "[]".into()),
                merge_status: match ticket.merge_status {
                    MergeStatus::Pending => "pending".to_string(),
                    MergeStatus::Merged => "merged".to_string(),
                    MergeStatus::Conflict => "conflict".to_string(),
                    MergeStatus::Skipped => "skipped".to_string(),
                },
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
                origin_session: None,
                base_commit: ticket.base_commit.clone(),
                blocked_by: serde_json::to_string(&ticket.blocked_by).unwrap_or_else(|_| "[]".into()),
                merge_status: match ticket.merge_status {
                    MergeStatus::Pending => "pending".to_string(),
                    MergeStatus::Merged => "merged".to_string(),
                    MergeStatus::Conflict => "conflict".to_string(),
                    MergeStatus::Skipped => "skipped".to_string(),
                },
            };
            if let Ok(db) = db.lock() {
                let _ = db.update_ticket(&row);
            }
        }
    }

    pub async fn submit_ticket(
        &self, title: String, prompt: String, context: Option<String>, criteria: Option<String>,
        team_mode: TeamMode, max_iterations: u16, blocked_by: Vec<usize>,
    ) -> Result<usize, String> {
        let mut guard = self.inner.write().await;
        // Validate all blocked_by IDs exist in current tickets
        for dep_id in &blocked_by {
            if !guard.tickets.iter().any(|t| t.id == *dep_id) {
                return Err(format!("blocked_by references unknown ticket id {}", dep_id));
            }
        }
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
            completed_elapsed_secs: None,
            base_commit: None,
            blocked_by,
            merge_status: MergeStatus::Pending,
        };
        guard.tickets.push(ticket.clone());
        drop(guard);
        self.persist_ticket(&ticket);
        Ok(id)
    }

    /// Check if a ticket's dependencies are all Done
    fn is_ready(ticket: &TaskTicket, all_tickets: &[TaskTicket]) -> bool {
        ticket.blocked_by.iter().all(|dep_id| {
            all_tickets.iter()
                .find(|t| t.id == *dep_id)
                .map(|t| t.status == TicketStatus::Done)
                .unwrap_or(true)
        })
    }

    pub async fn take_next(&self, worker_id: u16) -> Option<TicketSnapshot> {
        let mut guard = self.inner.write().await;
        let already_working = guard.tickets.iter().any(|t| {
            t.assigned_worker == Some(worker_id) && t.status == TicketStatus::Working
        });
        if already_working { return None; }

        // Find first Queued ticket whose dependencies are all Done
        let ready_id = {
            let tickets = &guard.tickets;
            tickets.iter()
                .find(|t| t.status == TicketStatus::Queued && Self::is_ready(t, tickets))
                .map(|t| t.id)
        };
        let ticket = match ready_id {
            Some(id) => guard.tickets.iter_mut().find(|t| t.id == id).unwrap(),
            None => return None,
        };
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

    /// Set the base commit for a ticket (captured when worker starts processing)
    pub async fn set_base_commit(&self, ticket_id: usize, commit: String) {
        let mut guard = self.inner.write().await;
        if let Some(ticket) = guard.tickets.iter_mut().find(|t| t.id == ticket_id) {
            ticket.base_commit = Some(commit);
            let snap = ticket.clone();
            drop(guard);
            self.persist_ticket_update(&snap);
        }
    }

    /// Update the merge status for a ticket
    pub async fn set_merge_status(&self, ticket_id: usize, status: MergeStatus) {
        let mut guard = self.inner.write().await;
        if let Some(ticket) = guard.tickets.iter_mut().find(|t| t.id == ticket_id) {
            ticket.merge_status = status;
            let snap = ticket.clone();
            drop(guard);
            self.persist_ticket_update(&snap);
        }
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
            ticket.completed_elapsed_secs = Some(ticket.elapsed_secs());
            ticket.status = TicketStatus::Done;
            ticket.summary = feedback;
            let snap = ticket.clone();
            drop(guard);
            self.persist_ticket_update(&snap);
            return false;
        }

        if ticket.iteration >= ticket.max_iterations {
            ticket.completed_elapsed_secs = Some(ticket.elapsed_secs());
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

    /// Force a ticket into Error status, bypassing retry logic (e.g. timeout)
    pub async fn force_error(&self, ticket_id: usize, reason: &str) {
        let mut guard = self.inner.write().await;
        if let Some(ticket) = guard.tickets.iter_mut().find(|t| t.id == ticket_id) {
            ticket.completed_elapsed_secs = Some(ticket.elapsed_secs());
            ticket.status = TicketStatus::Error;
            ticket.summary = Some(reason.to_string());
            let snap = ticket.clone();
            drop(guard);
            self.persist_ticket_update(&snap);
        }
    }

    /// Retry a failed ticket: reset Error → Queued for re-execution
    /// Retry a failed ticket, optionally updating its fields.
    pub async fn retry_ticket(
        &self, ticket_id: usize,
        new_prompt: Option<String>,
        new_context: Option<Option<String>>,
        new_criteria: Option<Option<String>>,
        feedback: Option<String>,
    ) -> bool {
        let mut guard = self.inner.write().await;
        let ticket = match guard.tickets.iter_mut().find(|t| t.id == ticket_id) {
            Some(t) => t,
            None => return false,
        };
        if ticket.status != TicketStatus::Error {
            return false;
        }
        if let Some(p) = new_prompt {
            ticket.prompt = p;
        }
        if let Some(c) = new_context {
            ticket.context = c;
        }
        if let Some(k) = new_criteria {
            ticket.criteria = k;
        }
        ticket.feedback = feedback;
        ticket.status = TicketStatus::Queued;
        ticket.assigned_worker = None;
        ticket.iteration = 0;
        ticket.summary = None;
        ticket.started_at = None;
        ticket.completed_elapsed_secs = None;
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
                ticket.completed_elapsed_secs = Some(ticket.elapsed_secs());
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
        base_commit: t.base_commit.clone(),
        blocked_by: t.blocked_by.clone(),
        merge_status: t.merge_status,
    }
}
