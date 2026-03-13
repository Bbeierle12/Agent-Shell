//! IPC message handlers for daemon commands and hook messages.
//!
//! Defines the [`DaemonCommand`] and [`DaemonResponse`] protocol, and the
//! [`handle_message`] function that dispatches incoming JSON to the correct
//! handler. Ported from ShellVault's `shellvault-daemon::handlers`.

use crate::state::AppState;
use agent_core::capture::types::HookMessage;
use chrono::Utc;
use serde::{Deserialize, Serialize};

/// Commands the daemon can receive over IPC.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "cmd")]
pub enum DaemonCommand {
    /// Ping / keepalive.
    #[serde(rename = "ping")]
    Ping,

    /// Get daemon status.
    #[serde(rename = "status")]
    Status,

    /// List active terminal sessions.
    #[serde(rename = "list_sessions")]
    ListSessions { limit: Option<u32> },

    /// Get details for a specific terminal session.
    #[serde(rename = "get_session")]
    GetSession { id: String },

    /// End a terminal session.
    #[serde(rename = "end_session")]
    EndSession { id: String },
}

/// Response sent back over IPC.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status")]
pub enum DaemonResponse {
    /// Successful response with optional payload.
    #[serde(rename = "ok")]
    Ok {
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },

    /// Error response.
    #[serde(rename = "error")]
    Error { message: String },
}

impl DaemonResponse {
    /// Shorthand for an Ok response with data.
    pub fn ok(data: serde_json::Value) -> Self {
        Self::Ok {
            data: Some(data),
        }
    }

    /// Shorthand for an Ok response without data.
    pub fn ok_empty() -> Self {
        Self::Ok { data: None }
    }

    /// Shorthand for an error response.
    pub fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
        }
    }
}

/// Handle an incoming IPC message.
///
/// Tries to parse the message as a [`HookMessage`] first (for shell hook
/// integration), then as a [`DaemonCommand`]. Returns a JSON string.
pub async fn handle_message(message: &str, state: &AppState) -> String {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return serde_json::to_string(&DaemonResponse::error("Empty message"))
            .unwrap_or_default();
    }

    // Try HookMessage first.
    if let Ok(hook_msg) = serde_json::from_str::<HookMessage>(trimmed) {
        return handle_hook_message(hook_msg, state).await;
    }

    // Try DaemonCommand.
    if let Ok(cmd) = serde_json::from_str::<DaemonCommand>(trimmed) {
        return handle_command(cmd, state).await;
    }

    serde_json::to_string(&DaemonResponse::error("Unknown message format"))
        .unwrap_or_default()
}

/// Process a shell hook message via the HookBackend.
async fn handle_hook_message(msg: HookMessage, state: &AppState) -> String {
    let event_type = msg.event_type();

    let mut backend = state.hook_backend.lock().await;
    match backend.process_message(msg).await {
        Ok(()) => {
            // Also feed the capture event to the terminal session manager
            // (events are sent through the channel, but we acknowledge here).
            let resp = DaemonResponse::ok(serde_json::json!({
                "event": format!("{:?}", event_type),
            }));
            serde_json::to_string(&resp).unwrap_or_default()
        }
        Err(e) => {
            tracing::error!("Failed to process hook message: {}", e);
            serde_json::to_string(&DaemonResponse::error(e.to_string()))
                .unwrap_or_default()
        }
    }
}

/// Handle a daemon command.
async fn handle_command(cmd: DaemonCommand, state: &AppState) -> String {
    let response = match cmd {
        DaemonCommand::Ping => DaemonResponse::ok(serde_json::json!({ "pong": true })),

        DaemonCommand::Status => {
            let tsm = state.terminal_sessions.read().await;
            let session_count = tsm.session_count();
            let uptime_secs = (Utc::now() - state.started_at).num_seconds().max(0) as u64;

            DaemonResponse::ok(serde_json::json!({
                "running": true,
                "uptime_secs": uptime_secs,
                "session_count": session_count,
                "version": env!("CARGO_PKG_VERSION"),
            }))
        }

        DaemonCommand::ListSessions { limit } => {
            let tsm = state.terminal_sessions.read().await;
            let sessions: Vec<serde_json::Value> = tsm
                .active_sessions()
                .into_iter()
                .take(limit.unwrap_or(50) as usize)
                .map(|s| {
                    serde_json::json!({
                        "id": s.id.to_string(),
                        "shell": s.shell,
                        "working_directory": s.working_directory.display().to_string(),
                        "started_at": s.started_at.to_rfc3339(),
                        "active": s.is_active(),
                    })
                })
                .collect();

            DaemonResponse::ok(serde_json::json!({
                "sessions": sessions,
                "count": sessions.len(),
            }))
        }

        DaemonCommand::GetSession { id } => {
            match uuid::Uuid::parse_str(&id) {
                Ok(uuid) => {
                    let tsm = state.terminal_sessions.read().await;
                    match tsm.get_session(&uuid) {
                        Some(session) => {
                            let commands: Vec<serde_json::Value> = tsm
                                .get_commands(&uuid)
                                .unwrap_or(&[])
                                .iter()
                                .map(|c| {
                                    serde_json::json!({
                                        "id": c.id.to_string(),
                                        "sequence": c.sequence,
                                        "command_text": c.command_text,
                                        "working_directory": c.working_directory.display().to_string(),
                                        "started_at": c.started_at.to_rfc3339(),
                                        "exit_code": c.exit_code,
                                        "duration_ms": c.duration_ms,
                                    })
                                })
                                .collect();

                            DaemonResponse::ok(serde_json::json!({
                                "session": {
                                    "id": session.id.to_string(),
                                    "shell": session.shell,
                                    "working_directory": session.working_directory.display().to_string(),
                                    "started_at": session.started_at.to_rfc3339(),
                                    "ended_at": session.ended_at.map(|t| t.to_rfc3339()),
                                    "terminal": session.terminal,
                                    "tags": session.tags,
                                    "active": session.is_active(),
                                },
                                "commands": commands,
                            }))
                        }
                        None => DaemonResponse::error("Session not found"),
                    }
                }
                Err(_) => DaemonResponse::error("Invalid session ID (expected UUID)"),
            }
        }

        DaemonCommand::EndSession { id } => {
            match uuid::Uuid::parse_str(&id) {
                Ok(uuid) => {
                    let mut tsm = state.terminal_sessions.write().await;
                    match tsm.get_session(&uuid) {
                        Some(_) => {
                            let end_event = agent_pty::CaptureEvent::SessionEnd {
                                session_id: uuid,
                                timestamp: Utc::now(),
                            };
                            tsm.process_event(&end_event);
                            DaemonResponse::ok_empty()
                        }
                        None => DaemonResponse::error("Session not found"),
                    }
                }
                Err(_) => DaemonResponse::error("Invalid session ID (expected UUID)"),
            }
        }
    };

    serde_json::to_string(&response).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::capture::HookBackend;
    use agent_core::terminal_session::TerminalSessionManager;
    use std::sync::Arc;
    use tokio::sync::{Mutex, RwLock};

    /// Build a minimal AppState for testing.
    fn test_state() -> AppState {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut config = agent_core::config::AppConfig::default();
        config.session.history_dir = Some(tmp.path().to_path_buf());
        let skill_indexer = Arc::new(agent_skills::SkillIndexer::new(tmp.path().join("skills")));

        // Leak the TempDir so it outlives the test.
        std::mem::forget(tmp);

        let registry = Arc::new(agent_core::tool_registry::ToolRegistry::new());
        let plugin_registry = Arc::new(RwLock::new(agent_plugins::PluginRegistry::new()));

        let mut hook_backend = HookBackend::new();
        hook_backend.start();

        AppState {
            config: config.clone(),
            tool_registry: registry.clone(),
            session_manager: Arc::new(RwLock::new(
                agent_core::session::SessionManager::new(&config).unwrap(),
            )),
            agent_loop: Arc::new(
                agent_core::agent_loop::AgentLoop::new(config, registry).unwrap(),
            ),
            plugin_registry,
            skill_indexer,
            hook_backend: Arc::new(Mutex::new(hook_backend)),
            terminal_sessions: Arc::new(RwLock::new(TerminalSessionManager::new())),
            started_at: Utc::now(),
        }
    }

    #[test]
    fn test_daemon_command_serde_roundtrip() {
        let commands = vec![
            DaemonCommand::Ping,
            DaemonCommand::Status,
            DaemonCommand::ListSessions { limit: Some(10) },
            DaemonCommand::ListSessions { limit: None },
            DaemonCommand::GetSession {
                id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            },
            DaemonCommand::EndSession {
                id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            },
        ];

        for cmd in &commands {
            let json = serde_json::to_string(cmd).unwrap();
            let parsed: DaemonCommand = serde_json::from_str(&json).unwrap();
            assert_eq!(*cmd, parsed, "Roundtrip failed for {:?}", cmd);
        }
    }

    #[test]
    fn test_daemon_response_serde_roundtrip() {
        let responses = vec![
            DaemonResponse::ok_empty(),
            DaemonResponse::ok(serde_json::json!({"pong": true})),
            DaemonResponse::error("something went wrong"),
        ];

        for resp in &responses {
            let json = serde_json::to_string(resp).unwrap();
            let parsed: DaemonResponse = serde_json::from_str(&json).unwrap();
            assert_eq!(*resp, parsed, "Roundtrip failed for {:?}", resp);
        }
    }

    #[tokio::test]
    async fn test_handle_ping() {
        let state = test_state();
        let response = handle_message(r#"{"cmd":"ping"}"#, &state).await;
        let parsed: DaemonResponse = serde_json::from_str(&response).unwrap();

        match parsed {
            DaemonResponse::Ok { data } => {
                let data = data.unwrap();
                assert_eq!(data["pong"], true);
            }
            other => panic!("Expected Ok, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_handle_status_returns_uptime() {
        let state = test_state();
        let response = handle_message(r#"{"cmd":"status"}"#, &state).await;
        let parsed: DaemonResponse = serde_json::from_str(&response).unwrap();

        match parsed {
            DaemonResponse::Ok { data } => {
                let data = data.unwrap();
                assert!(data["running"].as_bool().unwrap());
                assert!(data["uptime_secs"].is_number());
                assert_eq!(data["session_count"], 0);
                assert!(data["version"].is_string());
            }
            other => panic!("Expected Ok, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_handle_hook_message() {
        let state = test_state();
        let hook_json = r#"{
            "type": "session_start",
            "v": 1,
            "sid": "test-session",
            "pid": 1234,
            "shell": "bash",
            "cwd": "/home/user",
            "ts": 1700000000000
        }"#;

        let response = handle_message(hook_json, &state).await;
        let parsed: DaemonResponse = serde_json::from_str(&response).unwrap();

        match parsed {
            DaemonResponse::Ok { data } => {
                let data = data.unwrap();
                assert!(data["event"].as_str().unwrap().contains("Session"));
            }
            other => panic!("Expected Ok, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_list_sessions_empty() {
        let state = test_state();
        let response = handle_message(r#"{"cmd":"list_sessions"}"#, &state).await;
        let parsed: DaemonResponse = serde_json::from_str(&response).unwrap();

        match parsed {
            DaemonResponse::Ok { data } => {
                let data = data.unwrap();
                let sessions = data["sessions"].as_array().unwrap();
                assert!(sessions.is_empty());
                assert_eq!(data["count"], 0);
            }
            other => panic!("Expected Ok, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_unknown_message_returns_error() {
        let state = test_state();
        let response = handle_message(r#"{"foo":"bar"}"#, &state).await;
        let parsed: DaemonResponse = serde_json::from_str(&response).unwrap();

        match parsed {
            DaemonResponse::Error { message } => {
                assert!(message.contains("Unknown"));
            }
            other => panic!("Expected Error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_empty_message_returns_error() {
        let state = test_state();
        let response = handle_message("", &state).await;
        let parsed: DaemonResponse = serde_json::from_str(&response).unwrap();

        assert!(matches!(parsed, DaemonResponse::Error { .. }));
    }

    #[tokio::test]
    async fn test_get_session_not_found() {
        let state = test_state();
        let response = handle_message(
            r#"{"cmd":"get_session","id":"550e8400-e29b-41d4-a716-446655440000"}"#,
            &state,
        )
        .await;
        let parsed: DaemonResponse = serde_json::from_str(&response).unwrap();

        match parsed {
            DaemonResponse::Error { message } => {
                assert!(message.contains("not found"));
            }
            other => panic!("Expected Error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_get_session_invalid_uuid() {
        let state = test_state();
        let response =
            handle_message(r#"{"cmd":"get_session","id":"not-a-uuid"}"#, &state).await;
        let parsed: DaemonResponse = serde_json::from_str(&response).unwrap();

        match parsed {
            DaemonResponse::Error { message } => {
                assert!(message.contains("Invalid"));
            }
            other => panic!("Expected Error, got {:?}", other),
        }
    }
}
