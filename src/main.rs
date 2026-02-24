mod repl;

use agent_core::config::AppConfig;
use agent_core::scheduler::{Scheduler, ScheduledTask};
use agent_core::tool_registry::ToolRegistry;
use agent_plugins::PluginRegistry;
use agent_skills::SkillIndexer;
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
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

    // Initialize skill indexer from the skills directory.
    let skills_dir = AppConfig::data_dir().join("skills");
    let skill_indexer = Arc::new(SkillIndexer::new(&skills_dir));

    // Load skills index if the directory exists.
    if skills_dir.is_dir() {
        if let Err(e) = skill_indexer.reload() {
            tracing::warn!("Failed to load skills index: {}", e);
        }
    } else {
        tracing::debug!("Skills directory not found at {:?}, skipping", skills_dir);
    }

    // Build tool registry with all built-in tools.
    let mut registry = ToolRegistry::new();
    agent_tools::register_all(&mut registry, &config, Some(skill_indexer.clone()));
    let registry = Arc::new(registry);

    // Build plugin registry (empty for now — plugins register at startup).
    let plugin_registry = Arc::new(RwLock::new(PluginRegistry::new()));

    tracing::info!(
        "Loaded {} tools, model: {}, endpoint: {}",
        registry.len(),
        config.provider.model,
        config.provider.api_base,
    );

    // Spawn the scheduler as a background task if any schedules are configured.
    if !config.schedules.is_empty() {
        let state_path = AppConfig::data_dir().join("scheduler_state.json");
        let scheduler = Scheduler::new(config.schedules.clone(), state_path);
        let (sched_tx, mut sched_rx) = tokio::sync::mpsc::unbounded_channel();

        tokio::spawn(async move {
            scheduler.run(sched_tx).await;
        });

        tokio::spawn(async move {
            while let Some(task) = sched_rx.recv().await {
                match &task {
                    ScheduledTask::Prompt {
                        schedule_name,
                        prompt,
                        ..
                    } => {
                        tracing::info!(
                            "Scheduled task '{}' fired: prompt={}",
                            schedule_name,
                            &prompt[..prompt.len().min(80)]
                        );
                    }
                    ScheduledTask::Heartbeat {
                        schedule_name,
                        skill,
                        ..
                    } => {
                        tracing::info!(
                            "Scheduled task '{}' fired: heartbeat (skill={})",
                            schedule_name,
                            skill
                        );
                    }
                    ScheduledTask::Custom { schedule_name, .. } => {
                        tracing::info!("Scheduled task '{}' fired: custom", schedule_name);
                    }
                }
            }
        });

        tracing::info!(
            "Scheduler running with {} schedule(s)",
            config.schedules.len()
        );
    }

    match cli.command {
        Some(Commands::Serve { host, port }) => {
            if let Some(h) = host {
                config.server.host = h;
            }
            if let Some(p) = port {
                config.server.port = p;
            }
            agent_server::serve(config, registry, plugin_registry, skill_indexer).await?;
        }
        Some(Commands::Config { action }) => {
            handle_config_command(action, &config)?;
        }
        Some(Commands::Chat { session }) => {
            repl::run(config, registry, skill_indexer, session).await?;
        }
        None => {
            repl::run(config, registry, skill_indexer, None).await?;
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
