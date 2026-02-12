pub mod proxy;
pub mod session;
pub mod ipc;
pub mod squad;

pub use proxy::{ProxyConfig, ProxyServer};
pub use session::{ClaudeSession, discover_sessions, get_session_display_name, get_claude_dir};
