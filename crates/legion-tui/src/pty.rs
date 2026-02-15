//! PTY management for embedding Claude Code in ratatui
//!
//! Spawns Claude Code in a pseudo-terminal, reads output into a vt100 parser,
//! and provides a writer for sending keyboard input.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};

/// Shared parser state - accessed by reader thread and render loop
pub type SharedParser = Arc<Mutex<vt100::Parser>>;

/// Manages a PTY running Claude Code
pub struct PtyHandle {
    pub parser: SharedParser,
    master: Box<dyn portable_pty::MasterPty>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl PtyHandle {
    /// Spawn Claude Code in a PTY with the given size and environment
    ///
    /// When `use_proxy` is true, ANTHROPIC_BASE_URL routes requests through the proxy.
    /// When false (Default mode), Claude Code uses its own native auth (OAuth for Claude Max).
    pub fn spawn(
        rows: u16,
        cols: u16,
        proxy_port: u16,
        control_port: u16,
        dangerously_skip_permissions: bool,
        worker_id: Option<u16>,
        orchestrate_port: Option<u16>,
        system_prompt: Option<&str>,
        use_proxy: bool,
        working_dir: Option<&std::path::Path>,
        continue_session: bool,
    ) -> Result<Self> {
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to open PTY")?;

        let mut cmd = CommandBuilder::new("claude");
        if let Some(dir) = working_dir {
            cmd.cwd(dir);
        }
        if continue_session {
            cmd.arg("--continue");
        }
        if dangerously_skip_permissions {
            cmd.arg("--dangerously-skip-permissions");
        }
        // Workers: prevent Claude Code from asking user questions
        if worker_id.is_some() {
            cmd.args(["--disallowedTools", "AskUserQuestion"]);
        }
        if let Some(prompt) = system_prompt {
            cmd.args(["--append-system-prompt", prompt]);
        }
        if use_proxy {
            cmd.env(
                "ANTHROPIC_BASE_URL",
                format!("http://127.0.0.1:{}", proxy_port),
            );
        }
        cmd.env("LEGION_CONTROL_PORT", control_port.to_string());

        // Prepend cargo bin dir to PATH so legion-* tools are discoverable
        if let Ok(home) = std::env::var("HOME") {
            let cargo_bin = format!("{}/.cargo/bin", home);
            let current_path = std::env::var("PATH").unwrap_or_default();
            if !current_path.contains(&cargo_bin) {
                cmd.env("PATH", format!("{}:{}", cargo_bin, current_path));
            }
        }
        if let Some(wid) = worker_id {
            cmd.env("LEGION_WORKER_ID", wid.to_string());
            cmd.env("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "1");
        }
        if let Some(op) = orchestrate_port {
            cmd.env("LEGION_ORCHESTRATE_PORT", op.to_string());
        }

        // Leader only: inject statusLine hook to export context data
        if worker_id.is_none() {
            if let Ok(status_file) = setup_leader_statusline_hook() {
                cmd.args(["--settings", &status_file]);
            }
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .context("Failed to spawn claude in PTY")?;
        drop(pair.slave);

        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 1000)));

        // Reader thread: PTY stdout -> vt100 parser
        let reader = pair
            .master
            .try_clone_reader()
            .context("Failed to clone PTY reader")?;
        let parser_clone = Arc::clone(&parser);
        std::thread::spawn(move || {
            let mut reader = reader;
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut p) = parser_clone.lock() {
                            p.process(&buf[..n]);
                        }
                    }
                }
            }
        });

        let writer = pair
            .master
            .take_writer()
            .context("Failed to take PTY writer")?;

        Ok(Self {
            parser,
            master: pair.master,
            writer,
            child,
        })
    }

    /// Send bytes to the PTY (keyboard input)
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.writer
            .write_all(data)
            .context("Failed to write to PTY")?;
        self.writer.flush().ok();
        Ok(())
    }

    /// Kill the child process and its entire process group
    pub fn kill(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.child.process_id() {
            // Kill the entire process group (Claude Code + subprocesses)
            unsafe { libc::kill(-(pid as i32), libc::SIGTERM); }
            // Brief wait for graceful shutdown
            std::thread::sleep(std::time::Duration::from_millis(100));
            // Force kill if still alive
            unsafe { libc::kill(-(pid as i32), libc::SIGKILL); }
        }
        #[cfg(not(unix))]
        let _ = self.child.kill();
    }

    /// Resize the PTY and vt100 parser
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        // Resize the actual PTY so the child process sees the new size
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to resize PTY master")?;
        // Update vt100 parser to match
        if let Ok(mut p) = self.parser.lock() {
            p.set_size(rows, cols);
        }
        Ok(())
    }
}

/// Path to the leader status JSON file written by the statusLine hook.
pub const LEADER_STATUS_PATH: &str = "/tmp/legion-leader-status.json";

/// Create a statusLine hook script and a settings JSON file that references it.
/// Returns the path to the settings JSON file (for `--settings` arg).
fn setup_leader_statusline_hook() -> Result<String> {
    let hook_path = "/tmp/legion-statusline-hook.sh";
    let settings_path = "/tmp/legion-statusline-settings.json";

    // Write hook script: reads stdin JSON, writes to status file
    std::fs::write(hook_path, format!(
        "#!/bin/bash\ncat > {}\n", LEADER_STATUS_PATH
    )).context("Failed to write statusline hook")?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(hook_path, std::fs::Permissions::from_mode(0o755))
            .context("Failed to chmod statusline hook")?;
    }

    // Write settings JSON
    std::fs::write(settings_path, format!(
        r#"{{"statusLine":{{"type":"command","command":"{}"}}}}"#,
        hook_path
    )).context("Failed to write statusline settings")?;

    Ok(settings_path.to_string())
}

/// Check if a PTY shows an idle Claude Code prompt.
///
/// Strategy: scan the **bottom half** of the visible screen for a line starting
/// with `❯` (U+276F). Only checking the bottom half avoids false positives from
/// old prompts in scrollback that haven't been pushed off screen yet.
///
/// Claude Code's bottom area (in narrow panes, status wraps into many lines):
///   ❯ Try "fix lint errors"        ← prompt / hint line
///   ──────────────────────          ← separator
///   rainlei@MacBook-Pro-4          ← user info (may wrap!)
///   worker-1                       ← continuation
///   legion/session-2/worker-1      ← continuation
///   ⏵⏵ bypass permissions on ...   ← permissions
///   Claude in Chrome enabled       ← plugins
pub fn is_pty_idle(parser: &SharedParser) -> bool {
    if let Ok(p) = parser.lock() {
        let screen = p.screen();
        let (rows, cols) = screen.size();
        // Only check the bottom half of the screen to avoid old prompts in scrollback
        let start_row = rows / 2;
        let row_texts: Vec<String> = screen.rows(0, cols).collect();
        for row in row_texts.iter().skip(start_row as usize) {
            let trimmed = row.trim();
            if trimmed.starts_with('❯') {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: create a SharedParser and feed it lines of text
    fn make_screen(rows: u16, cols: u16, lines: &[&str]) -> SharedParser {
        let parser = Arc::new(Mutex::new(vt100::Parser::new(rows, cols, 0)));
        let mut p = parser.lock().unwrap();
        for (i, line) in lines.iter().enumerate() {
            // Move cursor to row i, col 0
            p.process(format!("\x1b[{};1H{}", i + 1, line).as_bytes());
        }
        drop(p);
        parser
    }

    #[test]
    fn test_idle_wide_pane() {
        // Wide pane: status bar fits on fewer lines, ❯ is near bottom
        let screen = make_screen(20, 80, &[
            "",
            "  Welcome back Rain!",
            "",
            "  Some previous output...",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "───────────────────────────────",
            "❯ Try \"fix lint errors\"",
            "───────────────────────────────",
            "  rainlei@MacBook-Pro-4 worker-1 legion/session-1/worker-1",
            "  ⏵⏵ bypass permissions on (shift+tab to cycle)",
            "",
            "",
        ]);
        assert!(is_pty_idle(&screen), "Should detect idle in wide pane");
    }

    #[test]
    fn test_idle_narrow_pane_wrapped_status() {
        // Narrow pane: status bar wraps, ❯ is many lines above bottom
        let screen = make_screen(20, 30, &[
            "",
            "  Welcome back!",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "──────────────────────",
            "❯ Try \"fix lint errors\"",
            "──────────────────────",
            "  rainlei@MacBook-Pro-4",
            "  worker-1",
            "  legion/session-2/worker-1",
            "  ⏵⏵ bypass permissions on",
            "  (shift+tab to cycle)",
            "  Claude in Chrome enabled",
            "",
        ]);
        assert!(is_pty_idle(&screen), "Should detect idle in narrow pane with wrapped status");
    }

    #[test]
    fn test_not_idle_claude_working() {
        // Claude is actively outputting, no ❯ in bottom half
        let screen = make_screen(20, 80, &[
            "❯ create hello.py",                 // old prompt - top half
            "",
            "⏺ I'll create hello.py for you.",
            "",
            "⏺ Write(hello.py)",
            "  def hello():",
            "      print(\"hello world\")",
            "",
            "⏺ Running tests...",
            "",
            "  $ python hello.py",               // bottom half starts here (row 10)
            "  hello world",
            "",
            "⏺ The file has been created.",
            "",
            "  Still working on next step...",
            "",
            "",
            "",
            "",
        ]);
        assert!(!is_pty_idle(&screen), "Should NOT detect idle when Claude is working (old ❯ in top half only)");
    }

    #[test]
    fn test_not_idle_empty_screen() {
        let screen = make_screen(20, 80, &[]);
        assert!(!is_pty_idle(&screen), "Empty screen should not be idle");
    }

    #[test]
    fn test_not_idle_gt_in_output() {
        // AI outputs > (greater-than), should NOT trigger
        let screen = make_screen(20, 80, &[
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "> This is a blockquote",
            ">> Nested quote",
            "  result > 0",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
        ]);
        assert!(!is_pty_idle(&screen), "Regular > should not trigger idle detection");
    }

    #[test]
    fn test_idle_prompt_with_input() {
        // Worker just received input, ❯ shows the typed text
        let screen = make_screen(20, 80, &[
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "───────────────────────────────",
            "❯ ",
            "───────────────────────────────",
            "  rainlei@MacBook-Pro-4 worker-1",
            "  ⏵⏵ bypass permissions on",
            "",
            "",
        ]);
        assert!(is_pty_idle(&screen), "Empty prompt ❯ should be idle");
    }
}
