//! HTTP client for agent-server API.
//!
//! Handles health checks, session management, and SSE streaming
//! for the chat completion endpoint.

use gloo_net::http::Request;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

/// Detect API base URL — use window.location origin in production,
/// fall back to localhost:3001 for dev.
pub fn detect_api_base() -> String {
    if let Some(window) = web_sys::window() {
        if let Ok(origin) = window.location().origin() {
            // If we're served from Trunk dev server (typically 8080),
            // proxy to agent-server on 3001.
            if origin.contains("127.0.0.1:8080") || origin.contains("localhost:8080") {
                return "http://127.0.0.1:3001".to_string();
            }
            return origin;
        }
    }
    "http://127.0.0.1:3001".to_string()
}

/// Read auth token from query string (?token=...) or localStorage.
pub fn get_auth_token() -> Option<String> {
    let window = web_sys::window()?;
    // Check query param first.
    if let Ok(search) = window.location().search() {
        for pair in search.trim_start_matches('?').split('&') {
            if let Some(val) = pair.strip_prefix("token=") {
                let token = val.to_string();
                // Persist to localStorage for future loads.
                if let Ok(Some(storage)) = window.local_storage() {
                    let _ = storage.set_item("agent_shell_token", &token);
                }
                return Some(token);
            }
        }
    }
    // Fall back to localStorage.
    if let Ok(Some(storage)) = window.local_storage() {
        if let Ok(Some(token)) = storage.get_item("agent_shell_token") {
            if !token.is_empty() {
                return Some(token);
            }
        }
    }
    None
}

/// Apply auth header to a web_sys::Headers if a token is available.
fn apply_auth(headers: &web_sys::Headers) {
    if let Some(token) = get_auth_token() {
        let _ = headers.set("Authorization", &format!("Bearer {}", token));
    }
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

/// A message from session history.
#[derive(Clone, Debug, Deserialize)]
pub struct HistoryMessage {
    pub id: String,
    pub role: String,
    pub content: String,
    pub tool_calls: Option<Vec<HistoryToolCall>>,
    pub tool_call_id: Option<String>,
    pub timestamp: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct HistoryToolCall {
    pub id: String,
    pub name: String,
}

/// Server config response.
#[derive(Clone, Debug, Deserialize)]
pub struct ServerConfig {
    pub provider: ProviderConfigInfo,
    pub server: ServerInfo,
    pub session: SessionConfigInfo,
    pub sandbox: SandboxInfo,
    pub tools: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProviderConfigInfo {
    pub api_base: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub top_p: f32,
    pub has_api_key: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServerInfo {
    pub host: String,
    pub port: u16,
    pub cors: bool,
    pub has_auth_token: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SessionConfigInfo {
    pub max_history: usize,
    pub auto_save: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SandboxInfo {
    pub mode: String,
    pub docker_image: String,
    pub timeout_secs: u64,
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

/// Apply optional auth header to a gloo-net RequestBuilder.
fn with_auth(req: gloo_net::http::RequestBuilder) -> gloo_net::http::RequestBuilder {
    if let Some(token) = get_auth_token() {
        req.header("Authorization", &format!("Bearer {}", token))
    } else {
        req
    }
}

/// Check if the backend is reachable.
pub async fn health_check(base: &str) -> Result<(), String> {
    let resp = with_auth(Request::get(&format!("{}/health", base)))
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
    let resp = with_auth(Request::get(&format!("{}/v1/sessions", base)))
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

    let resp = with_auth(Request::post(&format!("{}/v1/sessions", base)))
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

/// Fetch server configuration.
pub async fn get_config(base: &str) -> Result<ServerConfig, String> {
    let resp = with_auth(Request::get(&format!("{}/v1/config", base)))
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;

    if !resp.ok() {
        return Err(format!("Failed to get config: {}", resp.status()));
    }

    resp.json::<ServerConfig>()
        .await
        .map_err(|e| format!("Parse error: {}", e))
}

/// Fetch message history for a session.
pub async fn get_session_messages(base: &str, session_id: &str) -> Result<Vec<HistoryMessage>, String> {
    let resp = with_auth(Request::get(&format!("{}/v1/sessions/{}/messages", base, session_id)))
        .send()
        .await
        .map_err(|e| format!("Network error: {}", e))?;

    if !resp.ok() {
        return Err(format!("Failed to get messages: {}", resp.status()));
    }

    resp.json::<Vec<HistoryMessage>>()
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
    session_id: Option<&str>,
    on_event: impl Fn(StreamEvent) + 'static,
) -> Result<(), String> {
    let url = format!("{}/v1/chat/completions", base);
    let mut body = serde_json::json!({
        "messages": [{ "role": "user", "content": message }],
        "stream": true
    });
    if let Some(sid) = session_id {
        body["session_id"] = serde_json::json!(sid);
    }

    // Use web_sys Fetch API for streaming response body.
    let opts = web_sys::RequestInit::new();
    opts.set_method("POST");
    opts.set_mode(web_sys::RequestMode::Cors);

    let headers = web_sys::Headers::new().map_err(|_| "Failed to create headers")?;
    headers
        .set("Content-Type", "application/json")
        .map_err(|_| "Failed to set header")?;
    apply_auth(&headers);
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

/// Convert markdown text to HTML for display.
///
/// Handles fenced code blocks, inline code, bold, italic, headers, and lists.
/// This is intentionally minimal — no external crate dependency.
pub fn markdown_to_html(input: &str) -> String {
    let mut html = String::with_capacity(input.len() * 2);
    let mut in_code_block = false;
    let mut in_list = false;

    for line in input.lines() {
        // Fenced code blocks.
        if line.starts_with("```") {
            if in_code_block {
                html.push_str("</code></pre>");
                in_code_block = false;
            } else {
                if in_list {
                    html.push_str("</ul>");
                    in_list = false;
                }
                let lang = line.trim_start_matches('`').trim();
                if lang.is_empty() {
                    html.push_str("<pre><code>");
                } else {
                    html.push_str(&format!("<pre><code class=\"lang-{}\">", escape_html(lang)));
                }
                in_code_block = true;
            }
            continue;
        }
        if in_code_block {
            html.push_str(&escape_html(line));
            html.push('\n');
            continue;
        }

        let trimmed = line.trim();

        // Blank line — close list if open.
        if trimmed.is_empty() {
            if in_list {
                html.push_str("</ul>");
                in_list = false;
            }
            html.push_str("<br>");
            continue;
        }

        // Headers.
        if let Some(rest) = trimmed.strip_prefix("### ") {
            html.push_str(&format!("<h3>{}</h3>", inline_md(rest)));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            html.push_str(&format!("<h2>{}</h2>", inline_md(rest)));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("# ") {
            html.push_str(&format!("<h1>{}</h1>", inline_md(rest)));
            continue;
        }

        // Unordered list.
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            if !in_list {
                html.push_str("<ul>");
                in_list = true;
            }
            html.push_str(&format!("<li>{}</li>", inline_md(&trimmed[2..])));
            continue;
        }

        // Close list if this line isn't a list item.
        if in_list {
            html.push_str("</ul>");
            in_list = false;
        }

        // Normal paragraph line.
        html.push_str(&format!("<p>{}</p>", inline_md(trimmed)));
    }

    if in_code_block {
        html.push_str("</code></pre>");
    }
    if in_list {
        html.push_str("</ul>");
    }

    html
}

/// Process inline markdown: bold, italic, inline code.
fn inline_md(text: &str) -> String {
    let escaped = escape_html(text);
    // Inline code first (so bold/italic don't interfere inside backticks).
    let mut result = String::new();
    let mut parts = escaped.split('`');
    if let Some(first) = parts.next() {
        result.push_str(&bold_italic(first));
    }
    let mut in_code = false;
    for part in parts {
        if in_code {
            result.push_str(&format!("<code>{}</code>", part));
        } else {
            result.push_str(&bold_italic(part));
        }
        in_code = !in_code;
    }
    result
}

fn bold_italic(text: &str) -> String {
    // **bold** then *italic*
    let mut s = text.to_string();
    // Bold: **...**
    while let Some(start) = s.find("**") {
        if let Some(end) = s[start + 2..].find("**") {
            let inner = &s[start + 2..start + 2 + end].to_string();
            s = format!("{}<strong>{}</strong>{}", &s[..start], inner, &s[start + 2 + end + 2..]);
        } else {
            break;
        }
    }
    // Italic: *...*
    while let Some(start) = s.find('*') {
        if let Some(end) = s[start + 1..].find('*') {
            let inner = &s[start + 1..start + 1 + end].to_string();
            s = format!("{}<em>{}</em>{}", &s[..start], inner, &s[start + 1 + end + 1..]);
        } else {
            break;
        }
    }
    s
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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
