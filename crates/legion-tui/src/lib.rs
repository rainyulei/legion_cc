//! Legion TUI - Embedded terminal interface with provider/model switching

pub mod app;
pub mod input;
pub mod pty;
pub mod ui;

use std::io;

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use app::App;
use input::{handle_key, handle_mouse, InputResult};
use ui::draw;

/// Run the full TUI with a single embedded Claude Code
pub async fn run(proxy_port: u16, control_port: u16) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and load providers from DB
    let mut app = App::new();
    app.load_from_db();

    // Calculate PTY size from terminal (minus header=1, footer=1, border=2)
    let size = terminal.size()?;
    app.term_size = (size.width, size.height);
    let pty_rows = size.height.saturating_sub(4);
    let pty_cols = size.width.saturating_sub(2);

    // Start Claude in single pane (no skip permissions - normal interactive flow)
    app.add_pane(pty_rows, pty_cols, proxy_port, control_port, "Claude Code".into(), false);

    // Main event loop
    let result = run_event_loop(&mut terminal, &mut app).await;

    // Restore terminal
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    result
}

/// Run the squad TUI with multiple embedded Claude Code panes
pub async fn run_squad(worker_count: u16, base_port: u16) -> Result<()> {
    // Setup terminal with mouse support for divider dragging
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app and load providers from DB
    let mut app = App::new();
    app.load_from_db();

    // Cache terminal size
    let size = terminal.size()?;
    app.term_size = (size.width, size.height);

    // Calculate PTY sizes based on layout
    let content_height = size.height.saturating_sub(2); // header + footer

    // Leader pane: leader_ratio% width
    let leader_width = (size.width as u32 * app.leader_ratio as u32 / 100) as u16;
    let leader_pty_rows = content_height.saturating_sub(2);
    let leader_pty_cols = leader_width.saturating_sub(2);

    // Worker panes: remaining width minus 1 for divider, height divided equally
    let worker_width = size.width.saturating_sub(leader_width).saturating_sub(1);
    let worker_height = content_height / worker_count;
    let worker_pty_rows = worker_height.saturating_sub(2);
    let worker_pty_cols = worker_width.saturating_sub(2);

    // Port assignments:
    // Leader: proxy = base_port, control = base_port + 1000
    // Worker i: proxy = base_port + i + 1, control = base_port + 1000 + i + 1
    let leader_proxy = base_port;
    let leader_control = base_port + 1000;
    // Squad mode: all panes skip permissions (auto-trust)
    app.add_pane(leader_pty_rows, leader_pty_cols, leader_proxy, leader_control, "Leader".into(), true);

    for i in 0..worker_count {
        let proxy = base_port + i + 1;
        let control = base_port + 1000 + i + 1;
        let label = format!("Worker {}", i + 1);
        app.add_pane(worker_pty_rows, worker_pty_cols, proxy, control, label, true);
    }

    // Main event loop
    let result = run_event_loop(&mut terminal, &mut app).await;

    // Restore terminal
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture);
    let _ = terminal.show_cursor();

    result
}

/// Run the popup TUI only (for backward compat with `legion switch`)
pub async fn run_popup(control_port: u16) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new();
    app.load_from_db();
    app.toggle_popup(); // Start in popup mode

    let result = run_event_loop(&mut terminal, &mut app).await;

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    result
}

async fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|frame| draw(frame, app))?;

        if event::poll(std::time::Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    match handle_key(app, key) {
                        InputResult::Quit => break,
                        InputResult::Continue => {}
                    }
                }
                Event::Mouse(mouse) => {
                    handle_mouse(app, mouse);
                }
                Event::Resize(w, h) => {
                    app.resize_panes(w, h);
                }
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }
    Ok(())
}
