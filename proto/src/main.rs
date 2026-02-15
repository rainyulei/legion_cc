/// Prototype: test tui-term rendering of a complex TUI app (htop/top as proxy for Claude Code)
use std::io;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Terminal,
};
use tui_term::widget::PseudoTerminal;

fn main() -> Result<()> {
    // Determine terminal size
    let (cols, rows) = crossterm::terminal::size()?;

    // Allocate space for header(1) + border(2) + footer(1) = 4 lines overhead
    let pty_rows = rows.saturating_sub(4);
    let pty_cols = cols.saturating_sub(2); // border left + right

    // Create PTY
    let pty_system = NativePtySystem::default();
    let pair = pty_system.openpty(PtySize {
        rows: pty_rows,
        cols: pty_cols,
        pixel_width: 0,
        pixel_height: 0,
    })?;

    // Spawn a test command - use htop if available, otherwise top
    let mut cmd = CommandBuilder::new("htop");
    // If htop not available, you can change to "top" or "claude"
    let child = match pair.slave.spawn_command(cmd.clone()) {
        Ok(child) => child,
        Err(_) => {
            cmd = CommandBuilder::new("top");
            pair.slave.spawn_command(cmd)?
        }
    };
    drop(pair.slave); // close slave side

    // Create vt100 parser
    let parser = Arc::new(Mutex::new(vt100::Parser::new(pty_rows, pty_cols, 0)));

    // Reader thread: PTY -> vt100 parser
    let reader = pair.master.try_clone_reader()?;
    let parser_clone = Arc::clone(&parser);
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match std::io::Read::read(&mut reader, &mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    parser_clone.lock().unwrap().process(&buf[..n]);
                }
                Err(_) => break,
            }
        }
    });

    // Writer for sending input to PTY
    let mut writer = pair.master.take_writer()?;

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Main loop
    loop {
        terminal.draw(|frame| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1), // Header
                    Constraint::Min(0),    // Main (PTY)
                    Constraint::Length(1), // Footer
                ])
                .split(frame.area());

            // Header
            let header = Paragraph::new(Line::from(vec![
                Span::styled(
                    "Legion Proto",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("   "),
                Span::styled(
                    "[tui-term test]",
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            frame.render_widget(header, chunks[0]);

            // Main: PseudoTerminal widget
            let block = Block::default()
                .title(" Terminal Output ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue));

            let p = parser.lock().unwrap();
            let pseudo_term = PseudoTerminal::new(p.screen()).block(block);
            frame.render_widget(pseudo_term, chunks[1]);

            // Footer
            let footer = Paragraph::new(Line::from(vec![
                Span::styled("q", Style::default().fg(Color::Yellow)),
                Span::styled(": Quit  ", Style::default().fg(Color::DarkGray)),
                Span::styled("Ctrl+C", Style::default().fg(Color::Yellow)),
                Span::styled(": Stop", Style::default().fg(Color::DarkGray)),
            ]));
            frame.render_widget(footer, chunks[2]);
        })?;

        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                // Ctrl+Q to quit
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && key.code == KeyCode::Char('q')
                {
                    break;
                }
                // Forward all other keys to PTY
                let bytes = key_to_bytes(key);
                if !bytes.is_empty() {
                    let _ = std::io::Write::write_all(&mut writer, &bytes);
                }
            }
        }
    }

    // Cleanup
    drop(writer);
    drop(child);
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    Ok(())
}

/// Convert crossterm KeyEvent to bytes to send to PTY
fn key_to_bytes(key: crossterm::event::KeyEvent) -> Vec<u8> {
    use crossterm::event::{KeyCode, KeyModifiers};

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                // Ctrl+A = 0x01, Ctrl+B = 0x02, etc.
                vec![(c as u8).wrapping_sub(b'a').wrapping_add(1)]
            } else {
                c.to_string().into_bytes()
            }
        }
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![127],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Esc => vec![0x1b],
        KeyCode::Up => b"\x1b[A".to_vec(),
        KeyCode::Down => b"\x1b[B".to_vec(),
        KeyCode::Right => b"\x1b[C".to_vec(),
        KeyCode::Left => b"\x1b[D".to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        _ => vec![],
    }
}
