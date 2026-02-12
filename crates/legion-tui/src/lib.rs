//! Legion TUI - Terminal User Interface for Legion

pub mod app;
pub mod input;
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
pub async fn run_with_port(_port: u16) -> Result<()> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create app state
    let mut app = App::new();

    // Load data from database
    if let Ok(repo) = legion_db::open_db() {
        app.load_from_repo(&repo);
    }

    // Main event loop
    let result = run_event_loop(&mut terminal, &mut app).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

/// Main event loop
async fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        // Draw the UI
        terminal.draw(|frame| draw(frame, app))?;

        // Poll for events with a small timeout to allow async operations
        if event::poll(std::time::Duration::from_millis(100))? {
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
