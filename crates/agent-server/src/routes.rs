use crate::state::AppState;
use agent_core::types::{AgentEvent, Message};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Json};
use axum::routing::{get, post};
use axum::Router;
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
            let result = agent_loop
                .run(&messages, None, &[], tx)
                .await;
            if let Ok(msg) = result {
                let mut sm = session_manager.write().await;
                let _ = sm.push_message(msg);
            }
        });

        let stream = UnboundedReceiverStream::new(rx).map(|event| {
            let sse_event: Result<Event, std::convert::Infallible> = match event {
                AgentEvent::Token(token) => Ok(Event::default()
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
                AgentEvent::Error(e) => Ok(Event::default()
                    .event("error")
                    .data(e)),
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
    Router::new()
        .route("/v1/sessions", get(list_sessions).post(create_session))
}

#[derive(Debug, Serialize)]
struct SessionInfo {
    id: String,
    name: String,
    message_count: usize,
    updated_at: String,
}

async fn list_sessions(
    State(state): State<AppState>,
) -> impl IntoResponse {
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
