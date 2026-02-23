pub mod routes;
pub mod state;

use agent_core::config::AppConfig;
use agent_core::tool_registry::ToolRegistry;
use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub use state::AppState;

/// Middleware that validates a bearer token from the Authorization header.
async fn auth_middleware(
    State(state): axum::extract::State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let expected = match &state.config.server.auth_token {
        Some(t) => t,
        None => return next.run(req).await,
    };

    let auth_header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(value) if value.starts_with("Bearer ") && &value[7..] == expected => {
            next.run(req).await
        }
        _ => (StatusCode::UNAUTHORIZED, "Invalid or missing bearer token").into_response(),
    }
}

use axum::extract::State;

/// Build the axum Router with all routes and middleware.
pub fn build_router(state: AppState) -> Router {
    let config = &state.config;

    // Protected routes (chat, sessions) — require auth when token is configured.
    let protected = Router::new()
        .merge(routes::chat_routes())
        .merge(routes::session_routes())
        .route_layer(middleware::from_fn_with_state(state.clone(), auth_middleware));

    // Public routes (health) — never require auth.
    let public = Router::new().merge(routes::health_routes());

    let mut app = Router::new()
        .merge(protected)
        .merge(public)
        .with_state(state.clone());

    // Middleware stack.
    app = app.layer(TraceLayer::new_for_http());

    // CORS configuration.
    if config.server.cors {
        let cors = if config.server.auth_token.is_some() {
            // Restrictive CORS when auth is enabled.
            CorsLayer::new()
                .allow_methods([
                    axum::http::Method::GET,
                    axum::http::Method::POST,
                ])
                .allow_headers([
                    axum::http::header::CONTENT_TYPE,
                    axum::http::header::AUTHORIZATION,
                ])
                .allow_origin(Any)
        } else {
            // Permissive CORS for local dev (no auth).
            CorsLayer::permissive()
        };
        app = app.layer(cors);
    }

    app
}

/// Start the HTTP server.
pub async fn serve(config: AppConfig, tool_registry: Arc<ToolRegistry>) -> anyhow::Result<()> {
    let state = AppState::new(config.clone(), tool_registry);
    let router = build_router(state);

    let addr = format!("{}:{}", config.server.host, config.server.port);
    tracing::info!("Starting server on {}", addr);

    if config.server.auth_token.is_none() {
        tracing::warn!("No auth_token configured — server is unauthenticated!");
    }

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, router).await?;

    Ok(())
}
