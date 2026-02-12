use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "legion")]
#[command(about = "Claude Code wrapper with model switching and squad mode")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start legion in single window mode
    Start,
    /// Start legion in squad mode
    Squad {
        #[arg(short, long, default_value = "3")]
        workers: u32,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Start) | None => {
            legion_tui::run().await?;
        }
        Some(Commands::Squad { workers }) => {
            println!("Starting Legion squad mode with {} workers...", workers);
            // TODO: Start squad mode
        }
    }

    Ok(())
}
