//! HTTP client for agent-server API.
//!
//! Handles health checks, session management, and SSE streaming
//! for the chat completion endpoint.

use gloo_net::http::Request;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// Detect API base URL — use window.location origin in production,
/// fall back to localhost:3000 for dev.
pub fn detect_api_base() -> String {
    if let Some(window) = web_sys::window() {
        if let Ok(origin) = window.location().origin() {
            // If we're served from Trunk dev server (typically 8080),
            // proxy to agent-server on 3000.
            if origin.contains("127.0.0.1:8080") || origin.contains("localhost:8080") {
                return "http://127.0.0.1:3001".to_string();
            }
            return origin;
        }
    }
    "http://127.0.0.1:3001".to_string()
}

/// Session info returned by the sessions API.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub name: String,
    pub message_count: usize,
    pub updated_at: String,
}

/// Created session response.
#[derive(Clone, Debug, Deserialize)]
pub struct CreatedSession {
    pub id: String,
    pub name: String,
}

/// Events emitted during SSE streaming.
#[derive(Debug)]
pub enum StreamEvent {
    Token(String),
    ToolStart(String),
    ToolResult { content: String, is_error: bool },
    Done,
    Error(String),
}

/// Check if the backend is reachable.
pub async fn health_check(base: &str) -> Result<(), String> {
    let resp = Request::get(&format!("{}/health", base))
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;

    if resp.ok() {
        Ok(())
    } else {
        Err(format!("Health check failed: {}", resp.status()))
    }
}

/// List all sessions.
pub async fn list_sessions(base: &str) -> Result<Vec<SessionInfo>, String> {
    let resp = Request::get(&format!("{}/v1/sessions", base))
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;

    if !resp.ok() {
        return Err(format!("Failed to list sessions: {}", resp.status()));
    }

    resp.json::<Vec<SessionInfo>>()
        .await
        .map_err(|e| format!("Parse error: {}", e))
}

/// Create a new session.
pub async fn create_session(base: &str, name: &str) -> Result<CreatedSession, String> {
    let body = serde_json::json!({ "name": name });

    let resp = Request::post(&format!("{}/v1/sessions", base))
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .map_err(|e| format!("Request build error: {}", e))?
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;

    if !resp.ok() {
        return Err(format!("Failed to create session: {}", resp.status()));
    }

    resp.json::<CreatedSession>()
        .await
        .map_err(|e| format!("Parse error: {}", e))
}

/// Stream a chat completion via POST with SSE response.
///
/// Uses the Fetch API directly because browser EventSource only supports GET,
/// but our chat endpoint is POST.
pub async fn stream_chat(
    base: &str,
    message: &str,
    on_event: impl Fn(StreamEvent) + 'static,
) -> Result<(), String> {
    let url = format!("{}/v1/chat/completions", base);
    let body = serde_json::json!({
        "messages": [{ "role": "user", "content": message }],
        "stream": true
    });

    // Use web_sys Fetch API for streaming response body.
    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_mode(web_sys::RequestMode::Cors);

    let headers = web_sys::Headers::new().map_err(|_| "Failed to create headers")?;
    headers
        .set("Content-Type", "application/json")
        .map_err(|_| "Failed to set header")?;
    opts.set_headers(&headers);

    let js_body = JsValue::from_str(&body.to_string());
    opts.set_body(&js_body);

    let request = web_sys::Request::new_with_str_and_init(&url, &opts)
        .map_err(|_| "Failed to create request")?;

    let window = web_sys::window().ok_or("No window")?;
    let resp_value = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("Fetch failed: {:?}", e))?;

    let resp: web_sys::Response = resp_value
        .dyn_into()
        .map_err(|_| "Response cast failed")?;

    if !resp.ok() {
        return Err(format!("Chat request failed: {}", resp.status()));
    }

    let body = resp.body().ok_or("No response body")?;
    let reader = body
        .get_reader()
        .dyn_into::<web_sys::ReadableStreamDefaultReader>()
        .map_err(|_| "Failed to get reader")?;

    let text_decoder = web_sys::TextDecoder::new()
        .map_err(|_| "Failed to create TextDecoder")?;

    let mut buffer = String::new();

    loop {
        let result = JsFuture::from(reader.read())
            .await
            .map_err(|e| format!("Read error: {:?}", e))?;

        let done = web_sys::js_sys::Reflect::get(&result, &JsValue::from_str("done"))
            .map_err(|_| "No done field")?
            .as_bool()
            .unwrap_or(true);

        if done {
            on_event(StreamEvent::Done);
            break;
        }

        let value = web_sys::js_sys::Reflect::get(&result, &JsValue::from_str("value"))
            .map_err(|_| "No value field")?;

        let array: js_sys::Uint8Array = value.dyn_into().map_err(|_| "Not a Uint8Array")?;
        let chunk = text_decoder
            .decode_with_buffer_source(&array)
            .map_err(|_| "Decode error")?;

        buffer.push_str(&chunk);

        // Parse complete SSE frames from buffer (separated by \n\n).
        while let Some(pos) = buffer.find("\n\n") {
            let frame = buffer[..pos].to_string();
            buffer = buffer[pos + 2..].to_string();

            if let Some(event) = parse_sse_frame(&frame) {
                on_event(event);
            }
        }

        // Handle single newline-terminated lines.
        while buffer.contains('\n') {
            if let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].to_string();
                let rest = buffer[pos + 1..].to_string();

                if line.starts_with("data:") || line.starts_with("event:") {
                    if rest.is_empty()
                        || rest.starts_with("event:")
                        || rest.starts_with("data:")
                        || rest.starts_with('\n')
                    {
                        buffer = rest;
                        if let Some(event) = parse_sse_frame(&line) {
                            on_event(event);
                        }
                    } else {
                        break;
                    }
                } else if line.starts_with(':') || line.is_empty() {
                    buffer = rest;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    }

    Ok(())
}

/// Parse a single SSE frame.
fn parse_sse_frame(frame: &str) -> Option<StreamEvent> {
    let mut event_type = None;
    let mut data = None;

    for line in frame.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            event_type = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data = Some(rest.trim().to_string());
        }
    }

    let data = data?;

    if data == "[DONE]" {
        return Some(StreamEvent::Done);
    }

    match event_type.as_deref() {
        Some("tool_call") => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&data) {
                let name = val
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                Some(StreamEvent::ToolStart(name))
            } else {
                None
            }
        }
        Some("tool_result") => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&data) {
                let content = val
                    .get("content")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let is_error = val
                    .get("is_error")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                Some(StreamEvent::ToolResult { content, is_error })
            } else {
                None
            }
        }
        Some("error") => Some(StreamEvent::Error(data)),
        None | Some(_) => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&data) {
                if let Some(token) = val
                    .pointer("/choices/0/delta/content")
                    .and_then(|v| v.as_str())
                {
                    Some(StreamEvent::Token(token.to_string()))
                } else {
                    None
                }
            } else {
                None
            }
        }
    }
}
