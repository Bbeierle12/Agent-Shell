mod repl;

use agent_core::config::AppConfig;
use agent_core::tool_registry::ToolRegistry;
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(
    name = "agent-shell",
    about = "A model-agnostic AI agent shell for local LLMs",
    version,
    author
)]
struct Cli {
    /// Path to config file (default: ~/.config/agent-shell/config.toml)
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    /// Override the model name
    #[arg(short, long, global = true)]
    model: Option<String>,

    /// Override the API base URL
    #[arg(long, global = true)]
    api_base: Option<String>,

    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start interactive chat (default)
    Chat {
        /// Session name to create or resume
        #[arg(short, long)]
        session: Option<String>,
    },

    /// Start the HTTP/WebSocket server
    Serve {
        /// Bind host
        #[arg(long)]
        host: Option<String>,
        /// Bind port
        #[arg(long)]
        port: Option<u16>,
    },

    /// Show or manage configuration
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show current configuration
    Show,
    /// Initialize default configuration file
    Init,
    /// Open config file path
    Path,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Set up tracing.
    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new(std::env::var("RUST_LOG").unwrap_or_else(|_| "agent_shell=info,warn".into()))
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    // Load config.
    let mut config = match &cli.config {
        Some(path) => AppConfig::load_from(path)?,
        None => AppConfig::load()?,
    };

    // Apply CLI overrides.
    if let Some(model) = &cli.model {
        config.provider.model = model.clone();
    }
    if let Some(api_base) = &cli.api_base {
        config.provider.api_base = api_base.clone();
    }

    // Build tool registry with all built-in tools.
    let mut registry = ToolRegistry::new();
    agent_tools::register_all(&mut registry, &config);
    let registry = Arc::new(registry);

    tracing::info!(
        "Loaded {} tools, model: {}, endpoint: {}",
        registry.len(),
        config.provider.model,
        config.provider.api_base,
    );

    match cli.command {
        Some(Commands::Serve { host, port }) => {
            if let Some(h) = host {
                config.server.host = h;
            }
            if let Some(p) = port {
                config.server.port = p;
            }
            agent_server::serve(config, registry).await?;
        }
        Some(Commands::Config { action }) => {
            handle_config_command(action, &config)?;
        }
        Some(Commands::Chat { session }) => {
            repl::run(config, registry, session).await?;
        }
        None => {
            repl::run(config, registry, None).await?;
        }
    }

    Ok(())
}

fn handle_config_command(action: Option<ConfigAction>, config: &AppConfig) -> Result<()> {
    match action {
        Some(ConfigAction::Show) | None => {
            let toml_str = toml::to_string_pretty(config)?;
            println!("{}", toml_str);
        }
        Some(ConfigAction::Init) => {
            let path = AppConfig::default_path();
            if path.exists() {
                println!("Config already exists at: {}", path.display());
            } else {
                config.save()?;
                println!("Created default config at: {}", path.display());
            }
        }
        Some(ConfigAction::Path) => {
            println!("{}", AppConfig::default_path().display());
        }
    }
    Ok(())
}
