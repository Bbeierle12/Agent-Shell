use agent_core::agent_loop::AgentLoop;
use agent_core::capture::HookBackend;
use agent_core::config::AppConfig;
use agent_core::session::SessionManager;
use agent_core::terminal_session::TerminalSessionManager;
use agent_core::tool_registry::ToolRegistry;
use agent_plugins::PluginRegistry;
use agent_skills::SkillIndexer;
use chrono::{DateTime, Utc};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// Shared application state for the server.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<RwLock<AppConfig>>,
    pub tool_registry: Arc<ToolRegistry>,
    pub session_manager: Arc<RwLock<SessionManager>>,
    pub agent_loop: Arc<RwLock<AgentLoop>>,
    pub plugin_registry: Arc<RwLock<PluginRegistry>>,
    pub skill_indexer: Arc<SkillIndexer>,
    /// Hook backend for processing shell hook IPC messages.
    pub hook_backend: Arc<Mutex<HookBackend>>,
    /// In-memory terminal session manager (fed by hook events).
    pub terminal_sessions: Arc<RwLock<TerminalSessionManager>>,
    /// Timestamp when the server started (for uptime calculation).
    pub started_at: DateTime<Utc>,
}

impl AppState {
    pub fn new(
        config: AppConfig,
        tool_registry: Arc<ToolRegistry>,
        plugin_registry: Arc<RwLock<PluginRegistry>>,
        skill_indexer: Arc<SkillIndexer>,
    ) -> anyhow::Result<Self> {
        let session_manager = SessionManager::new(&config)?;
        let agent_loop = AgentLoop::new(config.clone(), tool_registry.clone())?;

        let mut hook_backend = HookBackend::new();
        hook_backend.start();

        Ok(Self {
            config: Arc::new(RwLock::new(config)),
            tool_registry,
            session_manager: Arc::new(RwLock::new(session_manager)),
            agent_loop: Arc::new(RwLock::new(agent_loop)),
            plugin_registry,
            skill_indexer,
            hook_backend: Arc::new(Mutex::new(hook_backend)),
            terminal_sessions: Arc::new(RwLock::new(TerminalSessionManager::new())),
            started_at: Utc::now(),
        })
    }
}
