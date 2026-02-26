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
use subtle::ConstantTimeEq;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub use state::AppState;

/// Middleware that validates a bearer token from the Authorization header.
///
/// Uses constant-time comparison (`subtle::ConstantTimeEq`) to prevent
/// timing-based side-channel attacks that could leak the token.
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
        Some(value) if value.starts_with("Bearer ") => {
            let provided = &value[7..];
            // Constant-time comparison: both operands are compared in full,
            // regardless of where they first differ.
            if provided.as_bytes().ct_eq(expected.as_bytes()).into() {
                next.run(req).await
            } else {
                (StatusCode::UNAUTHORIZED, "Invalid or missing bearer token").into_response()
            }
        }
        _ => (StatusCode::UNAUTHORIZED, "Invalid or missing bearer token").into_response(),
    }
}

use axum::extract::State;

/// Build the axum Router with all routes and middleware.
pub fn build_router(state: AppState) -> Router {
    let config = &state.config;

    // Protected routes (chat, sessions, plugins) — require auth when token is configured.
    let protected = Router::new()
        .merge(routes::chat_routes())
        .merge(routes::session_routes())
        .merge(routes::session_message_routes())
        .merge(routes::config_routes())
        .merge(routes::plugin_routes())
        .merge(routes::skill_routes())
        .merge(routes::terminal_routes())
        .merge(routes::context_routes())
        .merge(routes::analytics_routes())
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    // Public routes (health) — never require auth.
    let public = Router::new().merge(routes::health_routes());

    let spa = routes::spa_routes();

    let mut app = Router::new()
        .merge(protected)
        .merge(public)
        .merge(spa)
        .with_state(state.clone());

    // Middleware stack.
    app = app.layer(TraceLayer::new_for_http());

    // CORS configuration.
    if config.server.cors {
        let cors = if config.server.auth_token.is_some() {
            // Restrictive CORS when auth is enabled.
            CorsLayer::new()
                .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
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
pub async fn serve(
    config: AppConfig,
    tool_registry: Arc<ToolRegistry>,
    plugin_registry: Arc<tokio::sync::RwLock<agent_plugins::PluginRegistry>>,
    skill_indexer: Arc<agent_skills::SkillIndexer>,
) -> anyhow::Result<()> {
    let state = AppState::new(
        config.clone(),
        tool_registry,
        plugin_registry,
        skill_indexer,
    )?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    /// Build a test router with the given auth token and a temp session dir.
    fn test_router(auth_token: Option<String>) -> Router {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut config = AppConfig::default();
        config.server.auth_token = auth_token;
        config.session.history_dir = Some(tmp.path().to_path_buf());
        let skill_indexer = Arc::new(agent_skills::SkillIndexer::new(tmp.path().join("skills")));
        // Keep the TempDir alive by leaking it (tests are short-lived).
        std::mem::forget(tmp);

        let registry = Arc::new(ToolRegistry::new());
        let plugin_registry = Arc::new(tokio::sync::RwLock::new(
            agent_plugins::PluginRegistry::new(),
        ));
        let state = AppState::new(config, registry, plugin_registry, skill_indexer)
            .expect("Failed to create test app state");
        build_router(state)
    }

    #[tokio::test]
    async fn test_health_no_auth_required() {
        let app = test_router(Some("secret-token".into()));

        let req = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_protected_route_rejects_without_token() {
        let app = test_router(Some("secret-token".into()));

        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"messages":[{"role":"user","content":"hi"}]}"#,
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_protected_route_rejects_wrong_token() {
        let app = test_router(Some("secret-token".into()));

        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .header("authorization", "Bearer wrong-token")
            .body(Body::from(
                r#"{"messages":[{"role":"user","content":"hi"}]}"#,
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_protected_route_accepts_correct_token() {
        let app = test_router(Some("secret-token".into()));

        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .header("authorization", "Bearer secret-token")
            .body(Body::from(
                r#"{"messages":[{"role":"user","content":"hi"}]}"#,
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // Should NOT be 401 — the auth layer passed.
        // It may be 500 (no LLM backend) but that's fine for this test.
        assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_no_auth_allows_all() {
        let app = test_router(None);

        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header("content-type", "application/json")
            .body(Body::from(
                r#"{"messages":[{"role":"user","content":"hi"}]}"#,
            ))
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        // Without auth configured, requests should pass through (not 401).
        assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
