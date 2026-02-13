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
    _child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl PtyHandle {
    /// Spawn Claude Code in a PTY with the given size and environment
    pub fn spawn(
        rows: u16,
        cols: u16,
        proxy_port: u16,
        control_port: u16,
        dangerously_skip_permissions: bool,
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
        if dangerously_skip_permissions {
            cmd.arg("--dangerously-skip-permissions");
        }
        cmd.env(
            "ANTHROPIC_BASE_URL",
            format!("http://127.0.0.1:{}", proxy_port),
        );
        cmd.env("LEGION_CONTROL_PORT", control_port.to_string());

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
            _child: child,
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
