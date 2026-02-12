use anyhow::Result;
use clap::{Parser, Subcommand};
use legion_core::squad::SquadOrchestrator;

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
    Start {
        #[arg(long)]
        role: Option<String>,

        #[arg(long)]
        id: Option<String>,

        #[arg(long, default_value = "18080")]
        port: u16,
    },
    /// Start legion in squad mode
    Squad {
        #[arg(short, long, default_value = "3")]
        workers: u32,

        #[arg(long, default_value = "18080")]
        base_port: u16,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Start { role, id, port }) => {
            match role.as_deref() {
                Some("leader") => {
                    println!("Starting Legion as leader on port {}...", port);
                    legion_tui::run_with_port(port).await?;
                }
                Some("worker") => {
                    let worker_id = id.unwrap_or_else(|| "worker-1".to_string());
                    println!("Starting Legion as worker {} on port {}...", worker_id, port);
                    legion_tui::run_with_port(port).await?;
                }
                _ => {
                    legion_tui::run_with_port(port).await?;
                }
            }
        }
        Some(Commands::Squad { workers, base_port }) => {
            println!("Starting Legion squad mode with {} workers...", workers);
            let mut orchestrator = SquadOrchestrator::new(workers, base_port);
            orchestrator.start()?;
        }
        None => {
            legion_tui::run().await?;
        }
    }

    Ok(())
}
