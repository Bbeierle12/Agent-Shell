use agent_core::config::AppConfig;
use agent_core::tool_registry::ToolRegistry;
use agent_plugins::PluginRegistry;
use agent_skills::SkillIndexer;
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::RwLock;
use tracing_subscriber::EnvFilter;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Set up tracing.
    let filter =
        EnvFilter::new(std::env::var("RUST_LOG").unwrap_or_else(|_| "agent_desktop=info,warn".into()));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    // Load config.
    let mut config = AppConfig::load().unwrap_or_default();

    // Use a free port for the embedded server so it doesn't clash.
    config.server.host = "127.0.0.1".into();
    if config.server.port == 0 {
        config.server.port = 3001;
    }

    // Initialize skill indexer.
    let skills_dir = AppConfig::data_dir().join("skills");
    let skill_indexer = Arc::new(SkillIndexer::new(&skills_dir));
    if skills_dir.is_dir() {
        if let Err(e) = skill_indexer.reload() {
            tracing::warn!("Failed to load skills index: {}", e);
        }
    }

    // Build tool registry.
    let mut registry = ToolRegistry::new();
    agent_tools::register_all(&mut registry, &config, Some(skill_indexer.clone()));
    let registry = Arc::new(registry);

    // Build plugin registry.
    let plugin_registry = Arc::new(RwLock::new(PluginRegistry::new()));

    tracing::info!(
        "Agent Shell desktop: {} tools, model: {}, endpoint: {}",
        registry.len(),
        config.provider.model,
        config.provider.api_base,
    );

    let server_port = config.server.port;

    // Spawn the backend server in a background thread.
    let server_config = config.clone();
    let server_registry = registry.clone();
    let server_plugins = plugin_registry.clone();
    let server_skills = skill_indexer.clone();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
        rt.block_on(async {
            if let Err(e) =
                agent_server::serve(server_config, server_registry, server_plugins, server_skills)
                    .await
            {
                tracing::error!("Server error: {}", e);
            }
        });
    });

    tauri::Builder::default()
        .setup(move |app| {
            // Give the server a moment to bind.
            let port = server_port;
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(500));
                // Navigate the main window to the embedded server.
                if let Some(window) = handle.get_webview_window("main") {
                    let url = format!("http://127.0.0.1:{}", port);
                    tracing::info!("Navigating to {}", url);
                    let parsed: tauri::Url = url.parse().unwrap();
                    let _ = window.navigate(parsed);
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
