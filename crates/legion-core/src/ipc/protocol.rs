use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Risk {
    Low,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerStatus {
    Idle,
    Busy,
    Waiting,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingQuestion {
    pub id: i64,
    pub worker_id: String,
    pub risk: Risk,
    pub content: String,
    pub context: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Message {
    // Worker -> Daemon
    WorkerReady {
        worker_id: String,
        role: String,
    },
    Question {
        worker_id: String,
        risk: Risk,
        content: String,
        context: Option<String>,
    },
    StatusUpdate {
        worker_id: String,
        status: WorkerStatus,
        current_task: Option<String>,
    },

    // Daemon -> Worker
    Answer {
        question_id: i64,
        answer: String,
    },
    TaskAssign {
        task: String,
    },

    // Daemon -> Leader
    NewQuestion {
        question: PendingQuestion,
    },
    WorkerStatusChanged {
        worker_id: String,
        status: WorkerStatus,
    },
    AllWorkersReady {
        count: usize,
    },

    // Generic
    Ping,
    Pong,
    Error {
        message: String,
    },
}

pub fn serialize_message(msg: &Message) -> Vec<u8> {
    let json = serde_json::to_string(msg).unwrap();
    let len = json.len() as u32;
    let mut buf = len.to_be_bytes().to_vec();
    buf.extend(json.as_bytes());
    buf
}

pub fn deserialize_message(data: &[u8]) -> Option<Message> {
    if data.len() < 4 {
        return None;
    }
    let len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if data.len() < 4 + len {
        return None;
    }
    serde_json::from_slice(&data[4..4 + len]).ok()
}
