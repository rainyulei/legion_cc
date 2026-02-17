//! SDK mode execution for workers
//!
//! Spawns Claude Code with `--output-format=stream-json` for structured progress tracking.
//! Parses stream-json output and provides both:
//! 1. Formatted ANSI text for the pane's vt100 parser (visual display)
//! 2. Structured ProgressEntry items for the Task List popup

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::pty::SharedParser;

/// A running SDK process handle
pub struct SdkHandle {
    child: Child,
    /// Receives parsed progress entries from the background reader
    pub entry_rx: mpsc::UnboundedReceiver<ProgressEntry>,
    /// Set to true when the process has exited
    pub finished: Arc<Mutex<bool>>,
    /// Whether the process exited successfully
    pub success: Arc<Mutex<Option<bool>>>,
    /// Stores the Result text from the SDK output for promise detection
    pub result_text: Arc<Mutex<Option<String>>>,
    /// Full log buffer — all formatted output lines, never truncated
    pub log_buffer: Arc<Mutex<Vec<String>>>,
}

impl SdkHandle {
    /// Spawn Claude Code in SDK stream-json mode
    ///
    /// `existing_log_buffer` — pass in the previous iteration's log buffer to preserve history.
    pub fn spawn(
        working_dir: &Path,
        prompt: &str,
        parser: SharedParser,
        use_proxy: bool,
        proxy_port: u16,
        system_prompt: Option<&str>,
        iteration: u16,
        feedback: Option<&str>,
        existing_log_buffer: Option<Arc<Mutex<Vec<String>>>>,
    ) -> anyhow::Result<Self> {
        // Build the effective prompt with feedback from previous iterations
        let effective_prompt = if iteration > 1 {
            if let Some(fb) = feedback {
                format!(
                    "{}\n\n--- Previous Iteration Feedback (attempt {}) ---\n{}\n--- End Feedback ---\n\nPlease address the feedback above and complete the task. Output <promise>DONE</promise> when finished.",
                    prompt, iteration - 1, fb
                )
            } else {
                prompt.to_string()
            }
        } else {
            prompt.to_string()
        };

        let mut cmd = Command::new("claude");
        cmd.arg("-p")
            .arg(&effective_prompt)
            .args(["--output-format", "stream-json"])
            .args(["--verbose"])
            .arg("--dangerously-skip-permissions")
            .args(["--disallowedTools", "AskUserQuestion"])
            .current_dir(working_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null());

        if let Some(sp) = system_prompt {
            cmd.args(["--append-system-prompt", sp]);
        }

        if use_proxy {
            cmd.env(
                "ANTHROPIC_BASE_URL",
                format!("http://127.0.0.1:{}", proxy_port),
            );
        }

        // Prepend cargo bin dir to PATH
        if let Ok(home) = std::env::var("HOME") {
            let cargo_bin = format!("{}/.cargo/bin", home);
            let current_path = std::env::var("PATH").unwrap_or_default();
            if !current_path.contains(&cargo_bin) {
                cmd.env("PATH", format!("{}:{}", cargo_bin, current_path));
            }
        }

        let mut child = cmd.spawn()?;

        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        let (entry_tx, entry_rx) = mpsc::unbounded_channel();
        let finished = Arc::new(Mutex::new(false));
        let success = Arc::new(Mutex::new(None));
        let result_text = Arc::new(Mutex::new(None));
        let log_buffer: Arc<Mutex<Vec<String>>> = existing_log_buffer.unwrap_or_else(|| Arc::new(Mutex::new(Vec::new())));
        // Add iteration separator if this is a retry
        if iteration > 1 {
            if let Ok(mut buf) = log_buffer.lock() {
                buf.push(String::new());
                buf.push(format!("━━━ Iteration {} ━━━", iteration));
                buf.push(String::new());
            }
        }

        // Clear parser for fresh display
        {
            if let Ok(mut p) = parser.lock() {
                let (rows, cols) = p.screen().size();
                // Reset parser
                *p = vt100::Parser::new(rows, cols, crate::app::SCROLLBACK_LINES);
                // Write header
                let header = format!(
                    "\x1b[1;36m━━━ SDK Execution Mode ━━━\x1b[0m\r\n\r\n"
                );
                p.process(header.as_bytes());
            }
        }

        // Spawn stdout reader (stream-json parsing)
        let parser_clone = Arc::clone(&parser);
        let finished_clone = Arc::clone(&finished);
        let success_clone = Arc::clone(&success);
        let result_text_clone = Arc::clone(&result_text);
        let log_buffer_clone = Arc::clone(&log_buffer);
        let entry_tx_clone = entry_tx.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            let start = Instant::now();

            while let Ok(Some(line)) = lines.next_line().await {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                match serde_json::from_str::<ClaudeJson>(trimmed) {
                    Ok(msg) => {
                        let (formatted, entry) = process_message(&msg, start.elapsed().as_secs());
                        if let Some(ref text) = formatted {
                            if let Ok(mut p) = parser_clone.lock() {
                                p.process(text.as_bytes());
                            }
                            // Append clean text to log buffer (strip ANSI codes)
                            let clean = strip_ansi(text);
                            if !clean.trim().is_empty() {
                                if let Ok(mut buf) = log_buffer_clone.lock() {
                                    for line in clean.lines() {
                                        buf.push(line.to_string());
                                    }
                                }
                            }
                        }
                        if let Some(e) = entry {
                            let _ = entry_tx_clone.send(e);
                        }

                        // Check for Result message (execution complete)
                        if let ClaudeJson::Result { is_error, result, .. } = &msg {
                            // Store result text for promise detection
                            if let Ok(mut rt) = result_text_clone.lock() {
                                *rt = result.clone();
                            }
                            if let Ok(mut s) = success_clone.lock() {
                                *s = Some(!is_error.unwrap_or(false));
                            }
                            break;
                        }
                    }
                    Err(_) => {
                        // Unknown JSON line, write raw to parser
                        if let Ok(mut p) = parser_clone.lock() {
                            let text = format!("\x1b[90m{}\x1b[0m\r\n", truncate(trimmed, 120));
                            p.process(text.as_bytes());
                        }
                    }
                }
            }

            if let Ok(mut f) = finished_clone.lock() {
                *f = true;
            }
            // Send completion entry
            let elapsed = start.elapsed().as_secs();
            let ok = success_clone.lock().ok().and_then(|s| *s).unwrap_or(false);
            let _ = entry_tx_clone.send(ProgressEntry::Complete {
                duration_secs: elapsed,
                success: ok,
            });

            // Write completion to parser
            if let Ok(mut p) = parser_clone.lock() {
                let icon = if ok { "\x1b[1;32m✅" } else { "\x1b[1;31m❌" };
                let text = format!(
                    "\r\n{} Task {} ({}s)\x1b[0m\r\n",
                    icon,
                    if ok { "Complete" } else { "Failed" },
                    elapsed
                );
                p.process(text.as_bytes());
            }
        });

        // Spawn stderr reader
        let parser_clone2 = Arc::clone(&parser);
        let log_buffer_clone2 = Arc::clone(&log_buffer);
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    if let Ok(mut p) = parser_clone2.lock() {
                        let text = format!("\x1b[31m{}\x1b[0m\r\n", truncate(trimmed, 120));
                        p.process(text.as_bytes());
                    }
                    if let Ok(mut buf) = log_buffer_clone2.lock() {
                        buf.push(format!("[stderr] {}", trimmed));
                    }
                }
            }
        });

        Ok(Self {
            child,
            entry_rx,
            finished,
            success,
            result_text,
            log_buffer,
        })
    }

    /// Check if the SDK process has finished
    pub fn is_finished(&self) -> bool {
        self.finished.lock().ok().map(|f| *f).unwrap_or(false)
    }

    /// Check if the SDK process finished successfully
    pub fn is_success(&self) -> Option<bool> {
        self.success.lock().ok().and_then(|s| *s)
    }

    /// Kill the SDK process
    pub fn kill(&mut self) {
        let _ = self.child.start_kill();
    }

    /// Get the result text from the SDK output (if finished)
    pub fn result_text(&self) -> Option<String> {
        self.result_text.lock().ok().and_then(|r| r.clone())
    }

    /// Drain any pending progress entries
    pub fn drain_entries(&mut self) -> Vec<ProgressEntry> {
        let mut entries = Vec::new();
        while let Ok(entry) = self.entry_rx.try_recv() {
            entries.push(entry);
        }
        entries
    }
}

// --- Stream-JSON types (simplified from vibe-kanban) ---

#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)]
enum ClaudeJson {
    System {
        #[serde(default)]
        subtype: Option<String>,
        #[serde(default)]
        session_id: Option<String>,
        #[serde(default)]
        model: Option<String>,
    },
    Assistant {
        message: AssistantMessage,
        #[serde(default)]
        session_id: Option<String>,
    },
    User {
        message: serde_json::Value,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        tool_name: String,
        #[serde(default)]
        tool_data: Option<serde_json::Value>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        #[serde(default)]
        result: Option<serde_json::Value>,
        #[serde(default)]
        is_error: Option<bool>,
    },
    Result {
        #[serde(default)]
        subtype: Option<String>,
        #[serde(default)]
        is_error: Option<bool>,
        #[serde(default)]
        duration_ms: Option<u64>,
        #[serde(default)]
        result: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, serde::Deserialize)]
struct AssistantMessage {
    #[serde(default)]
    content: Option<serde_json::Value>,
}

/// Normalized progress entry for task list display
#[derive(Debug, Clone)]
pub enum ProgressEntry {
    Thinking(String),
    Message(String),
    ToolUse {
        tool_name: String,
        summary: String,
    },
    ToolResult {
        summary: String,
        is_error: bool,
    },
    Error(String),
    Complete {
        duration_secs: u64,
        success: bool,
    },
}

impl ProgressEntry {
    /// Short display string for the task list
    pub fn display(&self) -> String {
        match self {
            Self::Thinking(s) => format!("💭 {}", truncate(s, 50)),
            Self::Message(s) => format!("📝 {}", truncate(s, 50)),
            Self::ToolUse { tool_name, summary } => {
                let icon = tool_icon(tool_name);
                format!("{} {} {}", icon, tool_name, truncate(summary, 40))
            }
            Self::ToolResult { summary, is_error } => {
                let icon = if *is_error { "❌" } else { "✓" };
                format!("  {} {}", icon, truncate(summary, 50))
            }
            Self::Error(s) => format!("⚠ {}", truncate(s, 50)),
            Self::Complete { duration_secs, success } => {
                let icon = if *success { "✅" } else { "❌" };
                format!("{} {}s", icon, duration_secs)
            }
        }
    }
}

// --- Message processing ---

fn process_message(msg: &ClaudeJson, elapsed_secs: u64) -> (Option<String>, Option<ProgressEntry>) {
    match msg {
        ClaudeJson::System { model, session_id, .. } => {
            let model_str = model.as_deref().unwrap_or("unknown");
            // Only display system message if model is known (skip duplicate init messages)
            if model_str == "unknown" {
                (None, None)
            } else {
                let formatted = format!(
                    "\x1b[90m[{}s] Model: {} | Session: {}\x1b[0m\r\n",
                    elapsed_secs,
                    model_str,
                    &session_id.as_deref().unwrap_or("-")[..8.min(session_id.as_deref().unwrap_or("-").len())],
                );
                (Some(formatted), None)
            }
        }
        ClaudeJson::Assistant { message, .. } => {
            let content = extract_assistant_content(message);
            if content.is_empty() {
                return (None, None);
            }

            // Check if it's thinking content
            let is_thinking = message.content.as_ref()
                .and_then(|c| c.as_array())
                .map(|arr| arr.iter().any(|item| {
                    item.get("type").and_then(|t| t.as_str()) == Some("thinking")
                }))
                .unwrap_or(false);

            if is_thinking {
                let formatted = format!(
                    "\x1b[90m[{}s]\x1b[0m \x1b[2;33m💭 Thinking:\x1b[0m\r\n{}\r\n",
                    elapsed_secs,
                    wrap_text(&content, 120),
                );
                (Some(formatted), Some(ProgressEntry::Thinking(content)))
            } else {
                let formatted = format!(
                    "\x1b[90m[{}s]\x1b[0m \x1b[1;37m📝 Assistant:\x1b[0m\r\n{}\r\n",
                    elapsed_secs,
                    wrap_text(&content, 120),
                );
                (Some(formatted), Some(ProgressEntry::Message(content)))
            }
        }
        ClaudeJson::ToolUse { tool_name, tool_data } => {
            let summary = extract_tool_summary(tool_name, tool_data.as_ref());
            let icon = tool_icon(tool_name);
            let formatted = format!(
                "\x1b[90m[{}s]\x1b[0m \x1b[1;34m{} {}:\x1b[0m\r\n  \x1b[36m{}\x1b[0m\r\n",
                elapsed_secs,
                icon,
                tool_name,
                wrap_text(&summary, 120).trim_start(),
            );
            (
                Some(formatted),
                Some(ProgressEntry::ToolUse {
                    tool_name: tool_name.clone(),
                    summary,
                }),
            )
        }
        ClaudeJson::ToolResult { result, is_error } => {
            let is_err = is_error.unwrap_or(false);
            let summary = result
                .as_ref()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_default();

            let color = if is_err { "31" } else { "32" };
            let icon = if is_err { "❌" } else { "✓" };
            let formatted = if summary.trim().is_empty() {
                format!(
                    "  \x1b[{}m{}\x1b[0m\r\n",
                    color, icon,
                )
            } else {
                format!(
                    "  \x1b[{}m{} {}\x1b[0m\r\n",
                    color, icon, wrap_text(&summary, 120).trim_start(),
                )
            };
            (
                Some(formatted),
                Some(ProgressEntry::ToolResult {
                    summary,
                    is_error: is_err,
                }),
            )
        }
        ClaudeJson::Result { result, is_error, duration_ms, .. } => {
            let ok = !is_error.unwrap_or(false);
            let secs = duration_ms.unwrap_or(0) / 1000;
            let result_text = result.as_deref().unwrap_or("");
            let icon = if ok { "\x1b[1;32m✅" } else { "\x1b[1;31m❌" };
            let result_display = if result_text.is_empty() {
                String::new()
            } else {
                result_text.to_string()
            };
            let formatted = format!(
                "\r\n{} {} ({}s)\x1b[0m\r\n{}\r\n",
                icon,
                if ok { "Complete" } else { "Failed" },
                secs,
                result_display,
            );
            (
                Some(formatted),
                Some(ProgressEntry::Complete {
                    duration_secs: secs,
                    success: ok,
                }),
            )
        }
        _ => (None, None),
    }
}

fn extract_assistant_content(msg: &AssistantMessage) -> String {
    let content = match &msg.content {
        Some(c) => c,
        None => return String::new(),
    };

    // Content can be a string or array of content blocks
    if let Some(s) = content.as_str() {
        return s.to_string();
    }

    if let Some(arr) = content.as_array() {
        let mut parts = Vec::new();
        for item in arr {
            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                parts.push(text.to_string());
            }
            if let Some(thinking) = item.get("thinking").and_then(|t| t.as_str()) {
                parts.push(thinking.to_string());
            }
        }
        return parts.join("\n");
    }

    String::new()
}

fn extract_tool_summary(tool_name: &str, data: Option<&serde_json::Value>) -> String {
    let data = match data {
        Some(d) => d,
        None => return String::new(),
    };

    match tool_name {
        "Edit" | "Write" | "Read" => {
            data.get("file_path")
                .or_else(|| data.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        }
        "Bash" => {
            data.get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        }
        "Glob" | "Grep" => {
            data.get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        }
        "Task" => {
            data.get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        }
        _ => {
            // Generic: try to get a short summary
            let s = serde_json::to_string(data).unwrap_or_default();
            truncate(&s, 60).to_string()
        }
    }
}

fn tool_icon(tool_name: &str) -> &'static str {
    match tool_name {
        "Edit" | "Write" => "📝",
        "Read" => "📖",
        "Bash" => "🔧",
        "Glob" | "Grep" => "🔍",
        "Task" => "📋",
        _ => "⚙",
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.min(s.len())])
    }
}

fn wrap_text(s: &str, width: usize) -> String {
    let mut result = String::new();
    for line in s.lines() {
        if line.len() <= width {
            result.push_str("  ");
            result.push_str(line);
            result.push_str("\r\n");
        } else {
            let mut pos = 0;
            while pos < line.len() {
                let end = (pos + width).min(line.len());
                result.push_str("  ");
                result.push_str(&line[pos..end]);
                result.push_str("\r\n");
                pos = end;
            }
        }
    }
    result
}

// --- Promise detection for Ralph Loop ---

/// Check if SDK Result text contains the completion promise tag.
pub fn detect_promise(result_text: &str) -> bool {
    result_text.contains("<promise>DONE</promise>")
        || result_text.contains("<promise>COMPLETE</promise>")
}

/// Extract feedback from a non-promise Result text for Ralph Loop retry.
pub fn extract_feedback(result_text: &str) -> String {
    let cleaned = result_text
        .replace("<promise>", "")
        .replace("</promise>", "");
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "Task did not complete. No specific feedback provided.".to_string()
    } else {
        truncate(trimmed, 500)
    }
}

/// Strip ANSI escape sequences from a string, also convert \r\n to \n
fn strip_ansi(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' {
            // Skip ESC sequences: ESC [ ... <letter>
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&c) = chars.peek() {
                    chars.next();
                    if c.is_ascii_alphabetic() { break; }
                }
            }
        } else if ch == '\r' {
            // skip \r
        } else {
            result.push(ch);
        }
    }
    result
}
