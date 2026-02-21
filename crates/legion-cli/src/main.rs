use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use legion_core::proxy::{ProxyControlApi, ProxyServer};

#[derive(Parser)]
#[command(name = "legion")]
#[command(about = "Claude Code companion - multi-pane orchestration")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Base port (leader proxy port; others derived from this)
    #[arg(long, default_value = "18080")]
    base_port: u16,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new legion project in the current directory
    Init,
}

fn setup_logging(to_file: bool) {
    let filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive("legion=info".parse().unwrap())
        .add_directive("legion_core::proxy=debug".parse().unwrap());

    if to_file {
        let log_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("legion");
        let _ = fs::create_dir_all(&log_dir);
        let log_file = fs::File::create(log_dir.join("legion.log")).ok();
        if let Some(file) = log_file {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_writer(file)
                .with_ansi(false)
                .init();
            return;
        }
    }

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Init) => {
            setup_logging(false);
            cmd_init()?;
        }
        None => {
            setup_logging(true);
            cmd_squad(2, cli.base_port).await?;
        }
    }

    Ok(())
}

fn cmd_init() -> Result<()> {
    let cwd = std::env::current_dir()?;

    // Step 1: Git init
    let is_git = std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(&cwd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if is_git {
        println!("  ✓ Already a git repository");
    } else {
        let status = std::process::Command::new("git")
            .arg("init")
            .current_dir(&cwd)
            .status()?;
        if !status.success() {
            anyhow::bail!("git init failed");
        }
        // Create initial commit so worktrees work
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "Initial commit"])
            .current_dir(&cwd)
            .status()?;
        println!("  ✓ Git repository initialized");
    }

    // Step 2: Create .legion/ directory and project DB
    let legion_dir = cwd.join(".legion");
    fs::create_dir_all(&legion_dir)?;
    legion_db::open_project_db(&cwd)?; // creates and initializes DB
    println!("  ✓ Created .legion/ project database");

    // Add .legion/ to .gitignore
    let gitignore_path = cwd.join(".gitignore");
    let needs_entry = if gitignore_path.exists() {
        let content = fs::read_to_string(&gitignore_path)?;
        !content.lines().any(|l| l.trim() == ".legion/" || l.trim() == ".legion")
    } else {
        true
    };
    if needs_entry {
        use std::io::Write;
        let mut f = fs::OpenOptions::new().create(true).append(true).open(&gitignore_path)?;
        if f.metadata()?.len() > 0 {
            writeln!(f)?; // blank line separator
        }
        writeln!(f, ".legion/")?;
        println!("  ✓ Added .legion/ to .gitignore");
    }

    // Step 3: Create .claude/commands/
    let commands_dir = cwd.join(".claude").join("commands");
    fs::create_dir_all(&commands_dir)?;
    let split_tickets_path = commands_dir.join("split-tickets.md");
    if split_tickets_path.exists() {
        println!("  ✓ .claude/commands/split-tickets.md already exists, skipping");
    } else {
        legion_tui::claudemd::write_leader_commands(&cwd, 2)?;
        println!("  ✓ Created .claude/commands/split-tickets.md");
    }

    // Step 4: Write CLAUDE.md
    let claude_md_path = cwd.join("CLAUDE.md");
    let legion_section = r#"
## Legion

This project uses **Legion** for multi-agent orchestration with Claude Code.

### Squad Mode

- **Leader** — analyzes tasks, splits into tickets, dispatches to workers
- **Workers** — execute tickets independently in isolated git worktrees

### Available Tools

| Command | Description |
|---------|-------------|
| `/split-tickets` | Leader command: analyze task, create tickets, dispatch to workers |
| `legion-dispatch <worker> -t "title" -c "ctx" -k "criteria" "prompt"` | Dispatch a ticket to a worker |
| `legion-status` | Check worker status and ticket queue |
| `legion-check <ticket_id>` | Review a completed ticket's diff |
| `legion-report` | Get a summary report of all tickets |

### Workflow

1. Leader uses `/split-tickets` to decompose a task into independent tickets
2. Each ticket is dispatched to an available worker with `legion-dispatch`
3. Workers execute in isolated worktrees (no file conflicts)
4. Leader reviews with `legion-check` and merges completed work
"#;

    if claude_md_path.exists() {
        let content = fs::read_to_string(&claude_md_path)?;
        if content.contains("## Legion") {
            println!("  ✓ CLAUDE.md already has Legion section, skipping");
        } else {
            use std::io::Write;
            let mut f = fs::OpenOptions::new().append(true).open(&claude_md_path)?;
            writeln!(f, "{}", legion_section)?;
            println!("  ✓ Appended Legion section to existing CLAUDE.md");
        }
    } else {
        let project_name = cwd
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("project");
        let content = format!(
            r#"# {project_name}

## Project Overview

<!-- Describe your project here -->

## Tech Stack

<!-- List key technologies -->

## Development Guidelines

- Write tests for new features
- Keep commits focused and descriptive
- Follow existing code patterns and conventions

## Directory Structure

<!-- Document important directories -->
{legion_section}"#
        );
        fs::write(&claude_md_path, content)?;
        println!("  ✓ Created CLAUDE.md");
    }

    // Step 5: Import default providers (if DB is empty)
    let repo = legion_db::open_db()?;
    let providers = repo.list_providers()?;
    if providers.is_empty() {
        let now = chrono::Utc::now().timestamp();
        let defaults = vec![
            legion_db::Provider {
                id: "claude-code".into(),
                name: "Claude Code".into(),
                base_url: "https://api.anthropic.com".into(),
                api_key: None,
                api_format: "anthropic".into(),
                models: Some(vec![
                    "claude-opus-4-6".into(),
                    "claude-sonnet-4-5".into(),
                    "claude-haiku-4-5".into(),
                ]),
                is_default: false,
                created_at: now,
            },
            legion_db::Provider {
                id: "opencode-zen".into(),
                name: "OpenCode Zen".into(),
                base_url: "https://opencode.ai/zen/v1".into(),
                api_key: Some("free".into()),
                api_format: "anthropic_bearer".into(),
                models: None,
                is_default: true,
                created_at: now,
            },
        ];
        for p in &defaults {
            repo.upsert_provider(p)?;
        }
        println!("  ✓ Imported {} default providers", defaults.len());
    } else {
        println!("  ✓ Providers already configured ({} found)", providers.len());
    }

    println!("\nReady! Run `legion` to start.");
    Ok(())
}

/// Find an available base_port where key ports (base, base+1000, base+2000) are free.
fn find_available_base_port(preferred: u16) -> u16 {
    use std::net::TcpListener;
    let mut port = preferred;
    for _ in 0..100 {
        let ok = [port, port + 1000, port + 2000].iter().all(|p| {
            TcpListener::bind(("127.0.0.1", *p)).is_ok()
        });
        if ok { return port; }
        port += 100; // skip whole range
    }
    // Fallback: let OS pick
    TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap_or(preferred)
}

async fn cmd_squad(workers: u16, base_port: u16) -> Result<()> {
    let base_port = find_available_base_port(base_port);
    let mut default_config = None;
    let mut pane_configs: std::collections::HashMap<String, legion_core::ProxyConfig> =
        std::collections::HashMap::new();

    if let Ok(repo) = legion_db::open_db() {
        if let Ok(Some(provider)) = repo.get_default_provider() {
            default_config = Some(legion_core::ProxyConfig {
                target_url: Some(provider.base_url.clone()),
                api_key: provider.api_key.clone(),
                api_format: Some(provider.api_format.clone()),
                model: provider.models.as_ref().and_then(|m| m.first().cloned()),
            });
        }
        if let Ok(saved) = repo.list_pane_configs() {
            for pc in saved {
                if let Ok(Some(provider)) = repo.get_provider(&pc.provider_id) {
                    pane_configs.insert(
                        pc.pane_label,
                        legion_core::ProxyConfig {
                            target_url: Some(provider.base_url.clone()),
                            api_key: provider.api_key.clone(),
                            api_format: Some(provider.api_format.clone()),
                            model: pc.model,
                        },
                    );
                }
            }
        }
    }

    let total = 1 + workers;
    let mut ready_rxs = Vec::with_capacity(total as usize * 2);

    for idx in 0..total {
        let proxy_port = if idx == 0 { base_port } else { base_port + idx };
        let control_port = if idx == 0 { base_port + 1000 } else { base_port + 1000 + idx };

        let pane_label = if idx == 0 {
            "Leader".to_string()
        } else {
            format!("Worker {}", idx)
        };

        let proxy = ProxyServer::new(proxy_port);

        if let Some(config) = pane_configs.get(&pane_label) {
            proxy.update_config(config.clone()).await;
        } else if let Some(ref config) = default_config {
            proxy.update_config(config.clone()).await;
        }

        let (proxy_tx, proxy_rx) = tokio::sync::oneshot::channel();
        let proxy_config_ref = proxy.config_ref();
        tokio::spawn(async move {
            if let Err(e) = proxy.start_with_signal(Some(proxy_tx)).await {
                tracing::error!("Proxy error on port {}: {}", proxy_port, e);
            }
        });
        ready_rxs.push(proxy_rx);

        let (control_tx, control_rx) = tokio::sync::oneshot::channel();
        let control_api = ProxyControlApi::new(proxy_config_ref, control_port);
        tokio::spawn(async move {
            if let Err(e) = control_api.start_with_signal(Some(control_tx)).await {
                tracing::error!("Control API error on port {}: {}", control_port, e);
            }
        });
        ready_rxs.push(control_rx);
    }

    let timeout = std::time::Duration::from_secs(5);
    tokio::time::timeout(timeout, async {
        for rx in ready_rxs {
            rx.await.ok();
        }
    })
    .await
    .ok();

    legion_tui::run_squad(workers, base_port).await?;

    Ok(())
}
