//! PTY handling for embedded Claude Code process

use anyhow::Result;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

/// Handle to a PTY running Claude Code
pub struct PtyHandle {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    output_buffer: Arc<Mutex<Vec<u8>>>,
    _reader_handle: std::thread::JoinHandle<()>,
    _child: Box<dyn Child + Send + Sync>,
}

impl PtyHandle {
    /// Spawn Claude Code in a PTY with the given proxy port
    pub fn spawn_claude(proxy_port: u16) -> Result<Self> {
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })?;

        let mut cmd = CommandBuilder::new("claude");
        cmd.env(
            "ANTHROPIC_BASE_URL",
            format!("http://127.0.0.1:{}", proxy_port),
        );

        let child = pair.slave.spawn_command(cmd)?;
        drop(pair.slave); // Close slave end in parent

        let output_buffer = Arc::new(Mutex::new(Vec::new()));
        let buffer_clone = output_buffer.clone();

        let mut reader = pair.master.try_clone_reader()?;
        let writer = pair.master.take_writer()?;

        let reader_handle = std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let mut guard = buffer_clone.lock().unwrap();
                        guard.extend_from_slice(&buf[..n]);
                        // Keep buffer size reasonable
                        if guard.len() > 100_000 {
                            guard.drain(..50_000);
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            master: pair.master,
            writer,
            output_buffer,
            _reader_handle: reader_handle,
            _child: child,
        })
    }

    /// Write data to the PTY
    pub fn write(&mut self, data: &[u8]) -> Result<()> {
        self.writer.write_all(data)?;
        Ok(())
    }

    /// Read all output from the PTY buffer
    pub fn read_output(&self) -> Vec<u8> {
        let guard = self.output_buffer.lock().unwrap();
        guard.clone()
    }

    /// Resize the PTY
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }
}
