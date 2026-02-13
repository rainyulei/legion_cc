use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use legion_core::proxy::{ProxyControlApi, ProxyServer};

#[derive(Parser)]
#[command(name = "legion")]
#[command(about = "Claude Code companion - provider/model switching")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start legion TUI with embedded Claude Code
    Start {
        /// Proxy port
        #[arg(long, default_value = "18080")]
        port: u16,

        /// Control API port
        #[arg(long, default_value = "19080")]
        control_port: u16,

        /// Only start servers, no TUI (for testing)
        #[arg(long)]
        serve_only: bool,
    },

    /// Start squad mode with multiple Claude Code panes
    Squad {
        /// Number of worker panes (in addition to leader)
        #[arg(long, default_value = "2")]
        workers: u16,

        /// Base port (leader proxy port; others derived from this)
        #[arg(long, default_value = "18080")]
        base_port: u16,
    },

    /// Interactive provider/model switch (standalone popup)
    Switch {
        #[arg(long, default_value = "19080")]
        control_port: u16,
    },

    /// Import providers from cc-switch database
    Import {
        /// Path to cc-switch database (default: ~/.cc-switch/cc-switch.db)
        #[arg(long)]
        from: Option<PathBuf>,
    },
}

fn setup_logging(to_file: bool) {
    let filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive("legion=info".parse().unwrap());

    if to_file {
        // TUI mode: log to file so we don't corrupt the terminal
        let log_dir = dirs::data_local_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
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

    // serve-only / fallback: log to stderr
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Determine if we're running a TUI (need file logging) or serve-only (stderr OK)
    let use_file_logging = match &cli.command {
        Some(Commands::Start { serve_only, .. }) => !serve_only,
        None => true, // default is `start` with TUI
        _ => true,
    };
    setup_logging(use_file_logging);

    match cli.command {
        Some(Commands::Start {
            port,
            control_port,
            serve_only,
        }) => {
            cmd_start(port, control_port, serve_only).await?;
        }
        Some(Commands::Squad {
            workers,
            base_port,
        }) => {
            cmd_squad(workers, base_port).await?;
        }
        Some(Commands::Switch { control_port }) => {
            legion_tui::run_popup(control_port).await?;
        }
        Some(Commands::Import { from }) => {
            cmd_import(from)?;
        }
        None => {
            cmd_start(18080, 19080, false).await?;
        }
    }

    Ok(())
}

async fn cmd_start(proxy_port: u16, control_port: u16, serve_only: bool) -> Result<()> {
    let proxy = ProxyServer::new(proxy_port);

    // Configure proxy with default provider from DB
    if let Ok(repo) = legion_db::open_db() {
        if let Ok(Some(provider)) = repo.get_default_provider() {
            let config = legion_core::ProxyConfig {
                target_url: Some(provider.base_url.clone()),
                api_key: provider.api_key.clone(),
                api_format: Some(provider.api_format.clone()),
                model: provider.models.as_ref().and_then(|m| m.first().cloned()),
            };
            proxy.update_config(config).await;
        }
    }

    // Start proxy server
    let (proxy_tx, proxy_rx) = tokio::sync::oneshot::channel();
    let proxy_config_ref = proxy.config_ref();
    tokio::spawn(async move {
        if let Err(e) = proxy.start_with_signal(Some(proxy_tx)).await {
            tracing::error!("Proxy error: {}", e);
        }
    });

    // Start control API
    let (control_tx, control_rx) = tokio::sync::oneshot::channel();
    let control_api = ProxyControlApi::new(proxy_config_ref, control_port);
    tokio::spawn(async move {
        if let Err(e) = control_api.start_with_signal(Some(control_tx)).await {
            tracing::error!("Control API error: {}", e);
        }
    });

    // Wait for servers
    let timeout = std::time::Duration::from_secs(5);
    tokio::time::timeout(timeout, async {
        proxy_rx.await.ok();
        control_rx.await.ok();
    })
    .await
    .ok();

    if serve_only {
        println!(
            "Servers running - proxy :{}, control :{}",
            proxy_port, control_port
        );
        tokio::signal::ctrl_c().await?;
        return Ok(());
    }

    // Run the full TUI
    legion_tui::run(proxy_port, control_port).await?;

    Ok(())
}

fn cmd_import(from: Option<PathBuf>) -> Result<()> {
    let repo = legion_db::open_db()?;
    let now = chrono::Utc::now().timestamp();

    // Read Copilot token from cc-switch if available
    let copilot_token = read_ccswitch_copilot_token(&from);

    let providers = vec![
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
            models: None, // fetched dynamically below
            is_default: true,
            created_at: now,
        },
        legion_db::Provider {
            id: "github-copilot".into(),
            name: "GitHub Copilot".into(),
            base_url: "https://api.githubcopilot.com".into(),
            api_key: copilot_token.clone(),
            api_format: "anthropic_bearer".into(),
            models: None, // fetched dynamically below
            is_default: false,
            created_at: now,
        },
        legion_db::Provider {
            id: "codex-openai".into(),
            name: "Codex (OpenAI)".into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key: None,
            api_format: "openai_chat".into(),
            models: Some(vec![
                "gpt-5.2".into(),
                "gpt-5.2-codex".into(),
                "gpt-5.1-codex-max".into(),
            ]),
            is_default: false,
            created_at: now,
        },
    ];

    let mut imported = 0u32;
    let mut updated = 0u32;

    for provider in &providers {
        let action = if repo.get_provider(&provider.id)?.is_some() {
            "updated"
        } else {
            "imported"
        };
        repo.upsert_provider(provider)?;
        println!("  {}: {} [{}]", action, provider.name, provider.api_format);
        if action == "imported" { imported += 1; } else { updated += 1; }
    }

    // Dynamic model fetch for providers with tokens
    println!("\nFetching models...");

    // OpenCode Zen — always available (free)
    print!("  OpenCode Zen: ");
    match fetch_models_from_api("https://opencode.ai/zen/v1/models", "free") {
        Ok(models) => {
            println!("{} models", models.len());
            repo.update_provider_models("opencode-zen", &models)?;
        }
        Err(e) => println!("failed ({})", e),
    }

    // GitHub Copilot — fetch if we have a token
    if let Some(ref token) = copilot_token {
        print!("  GitHub Copilot: ");
        match fetch_models_from_api("https://api.githubcopilot.com/models", token) {
            Ok(models) => {
                println!("{} models", models.len());
                repo.update_provider_models("github-copilot", &models)?;
            }
            Err(e) => {
                println!("failed ({}), using defaults", e);
                let defaults: Vec<String> = vec![
                    "claude-sonnet-4-20250514".into(), "gpt-4o".into(),
                    "o3-mini".into(), "gemini-2.5-pro".into(),
                ];
                repo.update_provider_models("github-copilot", &defaults)?;
            }
        }
    }

    println!("\nDone: {} imported, {} updated", imported, updated);
    if copilot_token.is_none() {
        println!("Note: GitHub Copilot token not found (install cc-switch or set manually)");
    }
    Ok(())
}

/// Read GitHub Copilot token from cc-switch database
fn read_ccswitch_copilot_token(from: &Option<PathBuf>) -> Option<String> {
    let db_path = from.clone().unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".cc-switch")
            .join("cc-switch.db")
    });
    if !db_path.exists() {
        return None;
    }
    let conn = rusqlite::Connection::open(&db_path).ok()?;
    let token: Option<String> = conn
        .query_row(
            "SELECT json_extract(settings_config, '$.env.ANTHROPIC_AUTH_TOKEN') FROM providers WHERE app_type='claude' AND name LIKE '%Copilot%' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .ok()?;
    token
}

/// Fetch models from an OpenAI-compatible /models endpoint
fn fetch_models_from_api(url: &str, token: &str) -> Result<Vec<String>> {
    let body = ureq::get(url)
        .header("Authorization", format!("Bearer {}", token))
        .header("Copilot-Integration-Id", "vscode-chat")
        .call()
        .map_err(|e| anyhow::anyhow!("{}", e))?
        .into_body()
        .read_to_string()
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    let data: serde_json::Value = serde_json::from_str(&body)?;
    let models: Vec<String> = data["data"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|item| item["id"].as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    if models.is_empty() {
        anyhow::bail!("no models in response");
    }
    Ok(models)
}

async fn cmd_squad(workers: u16, base_port: u16) -> Result<()> {
    // Load default provider config from DB
    let default_config = if let Ok(repo) = legion_db::open_db() {
        if let Ok(Some(provider)) = repo.get_default_provider() {
            Some(legion_core::ProxyConfig {
                target_url: Some(provider.base_url.clone()),
                api_key: provider.api_key.clone(),
                api_format: Some(provider.api_format.clone()),
                model: provider.models.as_ref().and_then(|m| m.first().cloned()),
            })
        } else {
            None
        }
    } else {
        None
    };

    // Port assignments:
    // Leader: proxy = base_port, control = base_port + 1000
    // Worker i: proxy = base_port + i + 1, control = base_port + 1000 + i + 1
    let total = 1 + workers; // leader + workers
    let mut ready_rxs = Vec::with_capacity(total as usize * 2);

    for idx in 0..total {
        let proxy_port = if idx == 0 { base_port } else { base_port + idx };
        let control_port = if idx == 0 { base_port + 1000 } else { base_port + 1000 + idx };

        let proxy = ProxyServer::new(proxy_port);

        // Apply default config to each proxy
        if let Some(ref config) = default_config {
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

    // Wait for all servers to be ready
    let timeout = std::time::Duration::from_secs(5);
    tokio::time::timeout(timeout, async {
        for rx in ready_rxs {
            rx.await.ok();
        }
    })
    .await
    .ok();

    // Run the squad TUI
    legion_tui::run_squad(workers, base_port).await?;

    Ok(())
}
