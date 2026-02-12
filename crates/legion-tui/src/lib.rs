//! Legion TUI - Terminal User Interface for Legion

pub mod app;
pub mod input;
pub mod pty;
pub mod ui;

use std::io;

use anyhow::Result;
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use app::App;
use input::{handle_key, InputResult};
use ui::draw;

/// Run the TUI with default port
pub async fn run() -> Result<()> {
    run_with_port(18080).await
}

/// Run the TUI with specified port
pub async fn run_with_port(proxy_port: u16) -> Result<()> {
    // Setup terminal
    enable_raw_mode().map_err(|e| {
        anyhow::anyhow!(
            "Failed to initialize terminal: {}. Legion requires an interactive terminal (e.g. iTerm2, Terminal.app).",
            e
        )
    })?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state with proxy port
    let mut app = App::new(proxy_port);

    // Load data from database
    if let Ok(repo) = legion_db::open_db() {
        app.load_from_repo(&repo);
    }

    // Start proxy in background
    let proxy = app.proxy.clone();
    tokio::spawn(async move {
        if let Err(e) = proxy.start().await {
            tracing::error!("Proxy error: {}", e);
        }
    });

    // Wait a bit for proxy to start
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Connect to default provider if available
    if app.provider_connected {
        if let Err(e) = app.connect_provider().await {
            tracing::warn!("Failed to connect to default provider: {}", e);
        }
    }

    // Start Claude Code in PTY
    if let Err(e) = app.start_claude() {
        tracing::warn!("Failed to start Claude Code: {}", e);
    }

    // Main event loop
    let result = run_event_loop(&mut terminal, &mut app).await;

    // Restore terminal
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    result
}

/// Main event loop
async fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        // Update PTY output
        app.update_pty_output();

        // Draw the UI
        terminal.draw(|frame| draw(frame, app))?;

        // Poll for events with a small timeout for smoother PTY output
        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                match handle_key(app, key) {
                    InputResult::Quit => break,
                    InputResult::Continue => {}
                }
            }
        }

        // Check if app wants to quit
        if app.should_quit {
            break;
        }
    }

    Ok(())
}
