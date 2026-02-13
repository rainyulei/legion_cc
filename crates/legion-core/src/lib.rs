pub mod proxy;
pub mod session;
pub mod ipc;
pub mod squad;
pub mod launcher;

pub use proxy::{ProxyConfig, ProxyControlApi, ProxyServer};
pub use session::{ClaudeSession, discover_sessions, get_session_display_name, get_claude_dir};
pub use ipc::{Message, Risk, WorkerStatus, PendingQuestion, IpcClient, get_socket_path, serialize_message, deserialize_message};
pub use launcher::ClaudeLauncher;
