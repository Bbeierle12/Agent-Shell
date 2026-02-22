pub mod routes;
pub mod state;

use agent_core::config::AppConfig;
use agent_core::tool_registry::ToolRegistry;
use axum::Router;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

pub use state::AppState;

/// Build the axum Router with all routes and middleware.
pub fn build_router(state: AppState) -> Router {
    let mut app = Router::new()
        .merge(routes::chat_routes())
        .merge(routes::session_routes())
        .merge(routes::health_routes())
        .with_state(state.clone());

    // Middleware stack.
    app = app
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive());

    app
}

/// Start the HTTP server.
pub async fn serve(config: AppConfig, tool_registry: Arc<ToolRegistry>) -> anyhow::Result<()> {
    let state = AppState::new(config.clone(), tool_registry);
    let router = build_router(state);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    tracing::info!("Starting server on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}
