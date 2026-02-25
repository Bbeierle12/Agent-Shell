use crate::state::AppState;
use agent_core::context::ContextLinker;
use agent_core::types::{AgentEvent, Message};
use agent_plugins::PluginInfo;
use agent_pty::ShellInfo;
use agent_skills::SearchOptions;
use agent_tools::env_detect;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Json};
use axum::routing::{get, post};
use axum::Router;
use chrono::Datelike;
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::StreamExt;

// ── Health ──────────────────────────────────────────────────────────────

pub fn health_routes() -> Router<AppState> {
    Router::new().route("/health", get(health))
}

async fn health() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

// ── Chat ────────────────────────────────────────────────────────────────

pub fn chat_routes() -> Router<AppState> {
    Router::new().route("/v1/chat/completions", post(chat_completions))
}

#[derive(Debug, Deserialize)]
struct ChatRequest {
    messages: Vec<ChatMessage>,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    #[allow(dead_code)]
    session_id: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct ChatResponse {
    id: String,
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Serialize)]
struct ChatChoice {
    index: usize,
    message: ChatMessage,
    finish_reason: String,
}

async fn chat_completions(
    State(state): State<AppState>,
    Json(req): Json<ChatRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Convert API messages to our internal type.
    let user_msg = req
        .messages
        .last()
        .ok_or((StatusCode::BAD_REQUEST, "No messages provided".into()))?;

    let message = Message::user(&user_msg.content);

    // Add message to session.
    {
        let mut sm = state.session_manager.write().await;
        sm.push_message(message)
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    // Get message history.
    let messages: Vec<Message> = {
        let sm = state.session_manager.read().await;
        sm.recent_messages().into_iter().cloned().collect()
    };

    if req.stream {
        // SSE streaming response.
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();

        let agent_loop = state.agent_loop.clone();
        let session_manager = state.session_manager.clone();
        tokio::spawn(async move {
            let result = agent_loop.run(&messages, None, &[], tx.clone()).await;
            match result {
                Ok(msg) => {
                    let mut sm = session_manager.write().await;
                    let _ = sm.push_message(msg);
                }
                Err(e) => {
                    let _ = tx.send(AgentEvent::Error(e.to_string()));
                }
            }
        });

        let stream = UnboundedReceiverStream::new(rx).map(|event| {
            let sse_event: Result<Event, std::convert::Infallible> = match event {
                AgentEvent::ContentChunk(token) => Ok(Event::default()
                    .json_data(serde_json::json!({
                        "choices": [{"delta": {"content": token}}]
                    }))
                    .unwrap()),
                AgentEvent::ToolCallStart { name, .. } => Ok(Event::default()
                    .event("tool_call")
                    .json_data(serde_json::json!({"tool": name, "status": "started"}))
                    .unwrap()),
                AgentEvent::ToolResult(output) => Ok(Event::default()
                    .event("tool_result")
                    .json_data(serde_json::json!({
                        "tool_call_id": output.tool_call_id,
                        "content": output.content,
                        "is_error": output.is_error,
                    }))
                    .unwrap()),
                AgentEvent::Done(_) => Ok(Event::default().data("[DONE]")),
                AgentEvent::Error(e) => Ok(Event::default().event("error").data(e)),
                _ => Ok(Event::default().comment("ping")),
            };
            sse_event
        });

        Ok(Sse::new(stream).into_response())
    } else {
        // Non-streaming response.
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<AgentEvent>();

        let result = state
            .agent_loop
            .run(&messages, None, &[], tx)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        // Save assistant message.
        {
            let mut sm = state.session_manager.write().await;
            let _ = sm.push_message(result.clone());
        }

        let response = ChatResponse {
            id: result.id.clone(),
            choices: vec![ChatChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant".into(),
                    content: result.content.clone(),
                },
                finish_reason: "stop".into(),
            }],
        };

        Ok(Json(response).into_response())
    }
}

// ── Sessions ────────────────────────────────────────────────────────────

pub fn session_routes() -> Router<AppState> {
    Router::new().route("/v1/sessions", get(list_sessions).post(create_session))
}

#[derive(Debug, Serialize)]
struct SessionInfo {
    id: String,
    name: String,
    message_count: usize,
    updated_at: String,
}

async fn list_sessions(State(state): State<AppState>) -> impl IntoResponse {
    let sm = state.session_manager.read().await;
    let sessions: Vec<SessionInfo> = sm
        .list_sessions()
        .into_iter()
        .map(|(id, name, updated, count)| SessionInfo {
            id: id.to_string(),
            name: name.to_string(),
            message_count: count,
            updated_at: updated.to_rfc3339(),
        })
        .collect();
    Json(sessions)
}

#[derive(Debug, Deserialize)]
struct CreateSessionRequest {
    name: String,
}

// ── Config ─────────────────────────────────────────────────────────────

pub fn config_routes() -> Router<AppState> {
    Router::new().route("/v1/config", get(get_config))
}

#[derive(Debug, Serialize)]
struct ConfigResponse {
    provider: ProviderConfigResponse,
    server: ServerConfigResponse,
    session: SessionConfigResponse,
    sandbox: SandboxConfigResponse,
    tools: Vec<String>,
}

#[derive(Debug, Serialize)]
struct ProviderConfigResponse {
    api_base: String,
    model: String,
    max_tokens: u32,
    temperature: f32,
    top_p: f32,
    has_api_key: bool,
}

#[derive(Debug, Serialize)]
struct ServerConfigResponse {
    host: String,
    port: u16,
    cors: bool,
    has_auth_token: bool,
}

#[derive(Debug, Serialize)]
struct SessionConfigResponse {
    max_history: usize,
    auto_save: bool,
}

#[derive(Debug, Serialize)]
struct SandboxConfigResponse {
    mode: String,
    docker_image: String,
    timeout_secs: u64,
}

async fn get_config(State(state): State<AppState>) -> impl IntoResponse {
    let c = &state.config;
    let tools: Vec<String> = state
        .tool_registry
        .list_names()
        .iter()
        .map(|n| n.to_string())
        .collect();
    Json(ConfigResponse {
        provider: ProviderConfigResponse {
            api_base: c.provider.api_base.clone(),
            model: c.provider.model.clone(),
            max_tokens: c.provider.max_tokens,
            temperature: c.provider.temperature,
            top_p: c.provider.top_p,
            has_api_key: c.provider.api_key.is_some(),
        },
        server: ServerConfigResponse {
            host: c.server.host.clone(),
            port: c.server.port,
            cors: c.server.cors,
            has_auth_token: c.server.auth_token.is_some(),
        },
        session: SessionConfigResponse {
            max_history: c.session.max_history,
            auto_save: c.session.auto_save,
        },
        sandbox: SandboxConfigResponse {
            mode: format!("{:?}", c.sandbox.mode),
            docker_image: c.sandbox.docker_image.clone(),
            timeout_secs: c.sandbox.timeout_secs,
        },
        tools,
    })
}

// ── Session Messages ──────────────────────────────────────────────────

pub fn session_message_routes() -> Router<AppState> {
    Router::new().route("/v1/sessions/{id}/messages", get(get_session_messages))
}

async fn get_session_messages(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let sessions_dir = state
        .config
        .session
        .history_dir
        .clone()
        .unwrap_or_else(|| agent_core::config::AppConfig::data_dir().join("sessions"));

    let path = sessions_dir.join(format!("{}.json", id));
    let session = agent_core::session::Session::load_from(&path)
        .map_err(|e| (StatusCode::NOT_FOUND, format!("Session not found: {}", e)))?;

    #[derive(Serialize)]
    struct MessageResponse {
        id: String,
        role: String,
        content: String,
        tool_calls: Option<Vec<ToolCallResponse>>,
        tool_call_id: Option<String>,
        timestamp: String,
    }

    #[derive(Serialize)]
    struct ToolCallResponse {
        id: String,
        name: String,
    }

    let messages: Vec<MessageResponse> = session
        .messages
        .iter()
        .map(|m| MessageResponse {
            id: m.id.clone(),
            role: format!("{:?}", m.role).to_lowercase(),
            content: m.content.clone(),
            tool_calls: m.tool_calls.as_ref().map(|tcs| {
                tcs.iter()
                    .map(|tc| ToolCallResponse {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                    })
                    .collect()
            }),
            tool_call_id: m.tool_call_id.clone(),
            timestamp: m.timestamp.to_rfc3339(),
        })
        .collect();

    Ok(Json(messages))
}

// ── Plugins ────────────────────────────────────────────────────────────

pub fn plugin_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/plugins", get(list_plugins))
        .route("/v1/plugins/health", get(plugin_health))
}

async fn list_plugins(State(state): State<AppState>) -> impl IntoResponse {
    let pr = state.plugin_registry.read().await;
    let plugins: Vec<PluginInfo> = pr.list();
    Json(plugins)
}

#[derive(Debug, Serialize)]
struct PluginHealthEntry {
    category: String,
    name: String,
    status: String,
}

async fn plugin_health(State(state): State<AppState>) -> impl IntoResponse {
    let pr = state.plugin_registry.read().await;
    let entries: Vec<PluginHealthEntry> = pr
        .health_check_all()
        .into_iter()
        .map(|(key, status)| PluginHealthEntry {
            category: format!("{:?}", key.category),
            name: key.name,
            status: serde_json::to_value(&status)
                .unwrap_or_default()
                .as_str()
                .unwrap_or("unknown")
                .to_string(),
        })
        .collect();
    Json(entries)
}

// ── Skills ─────────────────────────────────────────────────────────────

pub fn skill_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/skills", get(list_skills))
        .route("/v1/skills/search", get(search_skills))
        .route("/v1/skills/{name}", get(get_skill))
}

#[derive(Debug, Serialize)]
struct SkillInfo {
    name: String,
    description: String,
    tags: Vec<String>,
    sub_skills: Vec<String>,
    source: Option<String>,
}

async fn list_skills(State(state): State<AppState>) -> impl IntoResponse {
    let index = state.skill_indexer.get_skill_index();
    let skills: Vec<SkillInfo> = index
        .skills
        .iter()
        .map(|s| SkillInfo {
            name: s.name.clone(),
            description: s.description.clone(),
            tags: s.tags.clone(),
            sub_skills: s.sub_skill_names().iter().map(|n| n.to_string()).collect(),
            source: s.source.clone(),
        })
        .collect();
    Json(skills)
}

#[derive(Debug, Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default = "default_limit")]
    limit: usize,
}

fn default_limit() -> usize {
    10
}

async fn search_skills(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<SearchQuery>,
) -> impl IntoResponse {
    let search = agent_skills::SearchService::new(state.skill_indexer.clone());
    let options = SearchOptions::with_limit(query.limit);
    let results = search.search_all(&query.q, &options);
    Json(results)
}

async fn get_skill(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let content = state
        .skill_indexer
        .read_skill_content(&name)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(content))
}

// ── Terminal ───────────────────────────────────────────────────────────

pub fn terminal_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/terminal/shells", get(list_shells))
        .route("/v1/terminal", get(terminal_ws))
}

async fn list_shells() -> impl IntoResponse {
    let shells: Vec<ShellInfo> = agent_pty::detect_available_shells();
    Json(shells)
}

/// WebSocket message from client.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum TerminalInput {
    /// Raw input data (base64-encoded bytes).
    Input { data: String },
    /// Resize the terminal.
    Resize { cols: u16, rows: u16 },
}

async fn terminal_ws(ws: axum::extract::WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_terminal_socket)
}

async fn handle_terminal_socket(mut socket: axum::extract::ws::WebSocket) {
    use axum::extract::ws::Message as WsMessage;
    use base64::Engine as _;

    let shell = match agent_pty::default_shell() {
        Some(s) => s,
        None => {
            let _ = socket
                .send(WsMessage::Text(
                    serde_json::json!({"type": "error", "message": "No shell found"})
                        .to_string()
                        .into(),
                ))
                .await;
            return;
        }
    };

    let session = match agent_pty::PtySession::new(&shell, 80, 24) {
        Ok(s) => s,
        Err(e) => {
            let _ = socket
                .send(WsMessage::Text(
                    serde_json::json!({"type": "error", "message": format!("PTY error: {e}")})
                        .to_string()
                        .into(),
                ))
                .await;
            return;
        }
    };

    // Send session info to client.
    let _ = socket
        .send(WsMessage::Text(
            serde_json::json!({
                "type": "session_start",
                "shell": shell.id,
                "cols": 80,
                "rows": 24,
            })
            .to_string()
            .into(),
        ))
        .await;

    let reader = session.reader();
    let session = std::sync::Arc::new(session);

    // Spawn a task to read PTY output and send to WebSocket.
    let (ws_tx, mut ws_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);

    // PTY reader task — runs in a blocking thread.
    let reader_clone = reader;
    tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; 4096];
        loop {
            let mut reader = reader_clone.blocking_lock();
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if ws_tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    tracing::debug!("PTY read error: {e}");
                    break;
                }
            }
        }
    });

    // Main loop: multiplex between PTY output → WS and WS input → PTY.
    loop {
        tokio::select! {
            // PTY output → WebSocket (binary).
            Some(data) = ws_rx.recv() => {
                if socket.send(WsMessage::Binary(data.into())).await.is_err() {
                    break;
                }
            }
            // WebSocket input → PTY.
            msg = socket.recv() => {
                match msg {
                    Some(Ok(WsMessage::Text(text))) => {
                        // JSON command (resize, etc.).
                        if let Ok(input) = serde_json::from_str::<TerminalInput>(&text) {
                            match input {
                                TerminalInput::Input { data } => {
                                    let engine = base64::engine::general_purpose::STANDARD;
                                    if let Ok(bytes) = engine.decode(&data) {
                                        let _ = session.write(&bytes).await;
                                    }
                                }
                                TerminalInput::Resize { cols, rows } => {
                                    let _ = session.resize(cols, rows);
                                }
                            }
                        }
                    }
                    Some(Ok(WsMessage::Binary(data))) => {
                        // Raw terminal input.
                        let _ = session.write(&data).await;
                    }
                    Some(Ok(WsMessage::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }

    tracing::debug!("Terminal WebSocket session closed");
}

// ── Context ───────────────────────────────────────────────────────────

pub fn context_routes() -> Router<AppState> {
    Router::new().route("/v1/context", get(get_context))
}

#[derive(Debug, Serialize)]
struct ContextResponse {
    project: Option<ProjectInfo>,
    git: Option<GitInfo>,
    environments: Vec<EnvInfo>,
}

#[derive(Debug, Serialize)]
struct ProjectInfo {
    name: String,
    project_type: String,
    path: String,
    git_remote: Option<String>,
    git_branch: Option<String>,
}

#[derive(Debug, Serialize)]
struct GitInfo {
    branch: Option<String>,
    remote: Option<String>,
    is_dirty: bool,
    head_short: Option<String>,
    repo_root: String,
}

#[derive(Debug, Serialize)]
struct EnvInfo {
    name: String,
    env_type: String,
    version: Option<String>,
    path: String,
}

async fn get_context(
    axum::extract::Query(params): axum::extract::Query<ContextQuery>,
) -> impl IntoResponse {
    let dir = params
        .directory
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        });

    let mut linker = ContextLinker::new();
    let project = linker.detect_project(&dir).map(|p| ProjectInfo {
        name: p.name.clone(),
        project_type: p.primary_type().display_name().to_string(),
        path: p.path.display().to_string(),
        git_remote: p.git_remote.clone(),
        git_branch: p.git_branch.clone(),
    });

    let git = ContextLinker::get_git_context(&dir).map(|g| GitInfo {
        branch: g.branch,
        remote: g.remote,
        is_dirty: g.is_dirty,
        head_short: g.head_short,
        repo_root: g.repo_root.display().to_string(),
    });

    let environments: Vec<EnvInfo> = env_detect::detect_environments(&dir)
        .into_iter()
        .map(|e| EnvInfo {
            name: e.name,
            env_type: e.env_type,
            version: e.version,
            path: e.path.display().to_string(),
        })
        .collect();

    Json(ContextResponse {
        project,
        git,
        environments,
    })
}

#[derive(Debug, Deserialize)]
struct ContextQuery {
    directory: Option<String>,
}

// ── Analytics ──────────────────────────────────────────────────────────

pub fn analytics_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/analytics/summary", get(analytics_summary))
        .route("/v1/analytics/report", get(analytics_report))
}

#[derive(Debug, Serialize)]
struct AnalyticsSummaryResponse {
    total_sessions: usize,
    active_days: usize,
    average_session_duration_secs: Option<u64>,
    top_tools: Vec<(String, u32)>,
    deep_work_sessions: usize,
    today: Option<DaySummaryResponse>,
}

#[derive(Debug, Serialize)]
struct DaySummaryResponse {
    sessions: u32,
    messages: u32,
    active_time: String,
    tool_calls: u32,
    tool_errors: u32,
}

async fn analytics_summary(State(state): State<AppState>) -> impl IntoResponse {
    let sm = state.session_manager.read().await;
    let sessions = sm.list_sessions();

    let mut analytics = agent_analytics::Analytics::default();

    // Load and process all sessions from disk.
    let sessions_dir = state
        .config
        .session
        .history_dir
        .clone()
        .unwrap_or_else(|| agent_core::config::AppConfig::data_dir().join("sessions"));

    for (id, _, _, _) in &sessions {
        let path = sessions_dir.join(format!("{}.json", id));
        if let Ok(session) = agent_core::session::Session::load_from(&path) {
            analytics.process_session(&session);
        }
    }
    analytics.finalize_all();

    let today = chrono::Utc::now().date_naive();
    let today_summary = analytics
        .get_daily_summary(today)
        .map(|s| DaySummaryResponse {
            sessions: s.session_count,
            messages: s.message_count,
            active_time: agent_analytics::aggregations::format_duration(s.total_active_time_secs),
            tool_calls: s.tool_call_count,
            tool_errors: s.tool_error_count,
        });

    Json(AnalyticsSummaryResponse {
        total_sessions: analytics.total_sessions(),
        active_days: analytics.active_days(),
        average_session_duration_secs: analytics.average_session_duration(),
        top_tools: analytics.top_tools(10),
        deep_work_sessions: analytics.deep_work_sessions().len(),
        today: today_summary,
    })
}

#[derive(Debug, Deserialize)]
struct ReportQuery {
    /// "week" or "month"
    #[serde(default = "default_period")]
    period: String,
}

fn default_period() -> String {
    "week".to_string()
}

async fn analytics_report(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<ReportQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let sm = state.session_manager.read().await;
    let sessions = sm.list_sessions();

    let mut analytics = agent_analytics::Analytics::default();

    let sessions_dir = state
        .config
        .session
        .history_dir
        .clone()
        .unwrap_or_else(|| agent_core::config::AppConfig::data_dir().join("sessions"));

    for (id, _, _, _) in &sessions {
        let path = sessions_dir.join(format!("{}.json", id));
        if let Ok(session) = agent_core::session::Session::load_from(&path) {
            analytics.process_session(&session);
        }
    }
    analytics.finalize_all();

    let today = chrono::Utc::now().date_naive();
    let report = match query.period.as_str() {
        "week" => {
            let weekday = today.weekday().num_days_from_monday();
            let monday = today - chrono::Duration::days(weekday as i64);
            agent_analytics::ReportGenerator::weekly_report(&analytics, monday)
        }
        "month" => agent_analytics::ReportGenerator::monthly_report(
            &analytics,
            today.year(),
            today.month(),
        ),
        other => {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Unknown period: '{}'. Use 'week' or 'month'.", other),
            ));
        }
    };

    Ok(report)
}

async fn create_session(
    State(state): State<AppState>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let mut sm = state.session_manager.write().await;
    let session = sm
        .create_session(req.name)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({
        "id": session.id,
        "name": session.name,
    })))
}

// ── SPA Static Files ────────────────────────────────────────────────────
//
// Serves the built React UI from crates/agent-ui/dist/.
// Falls back to index.html for all unmatched GET requests (client-side routing).
// The AGENT_UI_DIST env var overrides the default path.

pub fn spa_routes() -> Router<AppState> {
    use std::path::PathBuf;
    use tower_http::services::{ServeDir, ServeFile};

    let dist_path = std::env::var("AGENT_UI_DIST")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("crates/agent-ui/dist")
        });

    let index_path = dist_path.join("index.html");

    Router::new()
        .fallback_service(ServeDir::new(&dist_path).not_found_service(ServeFile::new(&index_path)))
}
