use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{mpsc, RwLock};

use legion_core::ipc::{
    deserialize_message, get_socket_path, serialize_message, Message, PendingQuestion, Risk,
    WorkerStatus,
};

pub struct WorkerState {
    pub id: String,
    pub role: String,
    pub status: WorkerStatus,
    pub current_task: Option<String>,
}

pub struct DaemonState {
    pub workers: HashMap<String, WorkerState>,
    pub pending_questions: Vec<PendingQuestion>,
    pub leader_tx: Option<mpsc::Sender<Message>>,
    pub next_question_id: i64,
}

impl DaemonState {
    pub fn new() -> Self {
        Self {
            workers: HashMap::new(),
            pending_questions: Vec::new(),
            leader_tx: None,
            next_question_id: 1,
        }
    }
}

pub struct DaemonServer {
    state: Arc<RwLock<DaemonState>>,
}

impl DaemonServer {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(DaemonState::new())),
        }
    }

    pub async fn start(&self) -> Result<()> {
        let socket_path = get_socket_path();

        // Remove existing socket
        let _ = std::fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path)?;
        tracing::info!("Daemon listening on {:?}", socket_path);

        loop {
            let (stream, _) = listener.accept().await?;
            let state = self.state.clone();

            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, state).await {
                    tracing::error!("Connection error: {}", e);
                }
            });
        }
    }
}

async fn handle_connection(
    mut stream: UnixStream,
    state: Arc<RwLock<DaemonState>>,
) -> Result<()> {
    let mut worker_id: Option<String> = None;

    loop {
        // Read message length
        let mut len_buf = [0u8; 4];
        if stream.read_exact(&mut len_buf).await.is_err() {
            break;
        }
        let len = u32::from_be_bytes(len_buf) as usize;

        // Read message body
        let mut data = vec![0u8; len];
        stream.read_exact(&mut data).await?;

        let mut full = len_buf.to_vec();
        full.extend(data);

        let msg = match deserialize_message(&full) {
            Some(m) => m,
            None => continue,
        };

        let response = process_message(msg, &state, &mut worker_id).await;

        if let Some(resp) = response {
            let resp_data = serialize_message(&resp);
            stream.write_all(&resp_data).await?;
        }
    }

    // Cleanup on disconnect
    if let Some(id) = worker_id {
        let mut guard = state.write().await;
        guard.workers.remove(&id);
    }

    Ok(())
}

async fn process_message(
    msg: Message,
    state: &Arc<RwLock<DaemonState>>,
    worker_id: &mut Option<String>,
) -> Option<Message> {
    match msg {
        Message::WorkerReady { worker_id: id, role } => {
            let mut guard = state.write().await;
            guard.workers.insert(
                id.clone(),
                WorkerState {
                    id: id.clone(),
                    role: role.clone(),
                    status: WorkerStatus::Idle,
                    current_task: None,
                },
            );
            *worker_id = Some(id);

            // Notify leader if all workers ready
            let worker_count = guard.workers.len();
            if let Some(tx) = &guard.leader_tx {
                let _ = tx
                    .send(Message::AllWorkersReady {
                        count: worker_count,
                    })
                    .await;
            }

            Some(Message::Pong)
        }

        Message::Question {
            worker_id: wid,
            risk,
            content,
            context,
        } => {
            let mut guard = state.write().await;
            let question_id = guard.next_question_id;
            guard.next_question_id += 1;

            let question = PendingQuestion {
                id: question_id,
                worker_id: wid,
                risk: risk.clone(),
                content,
                context,
            };

            match risk {
                Risk::Low => {
                    // Auto-answer low risk questions
                    Some(Message::Answer {
                        question_id,
                        answer: "y".to_string(),
                    })
                }
                Risk::High => {
                    // Forward to leader
                    guard.pending_questions.push(question.clone());
                    if let Some(tx) = &guard.leader_tx {
                        let _ = tx.send(Message::NewQuestion { question }).await;
                    }
                    None // No immediate response, wait for leader
                }
            }
        }

        Message::StatusUpdate {
            worker_id: wid,
            status,
            current_task,
        } => {
            let mut guard = state.write().await;
            if let Some(worker) = guard.workers.get_mut(&wid) {
                worker.status = status.clone();
                worker.current_task = current_task;
            }

            // Notify leader
            if let Some(tx) = &guard.leader_tx {
                let _ = tx
                    .send(Message::WorkerStatusChanged {
                        worker_id: wid,
                        status,
                    })
                    .await;
            }

            None
        }

        Message::Ping => Some(Message::Pong),

        _ => None,
    }
}
