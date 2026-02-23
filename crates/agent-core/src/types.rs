use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub role: Role,
    pub content: String,
    /// Tool call ID this message is responding to (for tool results).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Tool calls requested by the assistant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    pub timestamp: DateTime<Utc>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            role: Role::User,
            content: content.into(),
            tool_call_id: None,
            tool_calls: None,
            timestamp: Utc::now(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            role: Role::Assistant,
            content: content.into(),
            tool_call_id: None,
            tool_calls: None,
            timestamp: Utc::now(),
        }
    }

    pub fn assistant_with_tool_calls(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            role: Role::Assistant,
            content: content.into(),
            tool_call_id: None,
            tool_calls: Some(tool_calls),
            timestamp: Utc::now(),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            role: Role::System,
            content: content.into(),
            tool_call_id: None,
            tool_calls: None,
            timestamp: Utc::now(),
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            role: Role::Tool,
            content: content.into(),
            tool_call_id: Some(tool_call_id.into()),
            tool_calls: None,
            timestamp: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// A tool call requested by the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// Schema definition for a tool's parameters, sent to the model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Output from a tool execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    pub tool_call_id: String,
    pub content: String,
    pub is_error: bool,
}

/// Streaming event emitted during agent execution.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// A chunk of the assistant's response content.
    ContentChunk(String),
    /// The assistant is calling a tool.
    ToolCallStart { id: String, name: String },
    /// Tool call arguments (may arrive in chunks).
    ToolCallArgsChunk { id: String, chunk: String },
    /// Tool execution completed.
    ToolResult(ToolOutput),
    /// The full assistant message is complete.
    Done(Message),
    /// An error occurred.
    Error(String),
}
