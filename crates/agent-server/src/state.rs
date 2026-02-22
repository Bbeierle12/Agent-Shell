use agent_core::agent_loop::AgentLoop;
use agent_core::config::AppConfig;
use agent_core::session::SessionManager;
use agent_core::tool_registry::ToolRegistry;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared application state for the server.
#[derive(Clone)]
pub struct AppState {
    pub config: AppConfig,
    pub tool_registry: Arc<ToolRegistry>,
    pub session_manager: Arc<RwLock<SessionManager>>,
    pub agent_loop: Arc<AgentLoop>,
}

impl AppState {
    pub fn new(config: AppConfig, tool_registry: Arc<ToolRegistry>) -> Self {
        let session_manager = SessionManager::new(&config)
            .expect("Failed to initialize session manager");
        let agent_loop = AgentLoop::new(config.clone(), tool_registry.clone());

        Self {
            config,
            tool_registry,
            session_manager: Arc::new(RwLock::new(session_manager)),
            agent_loop: Arc::new(agent_loop),
        }
    }
}
