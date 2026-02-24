use thiserror::Error;

#[derive(Error, Debug)]
pub enum AgentError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Provider error: {0}")]
    Provider(String),

    #[error("Tool execution error: {tool_name}: {message}")]
    ToolExecution { tool_name: String, message: String },

    #[error("Tool not found: {0}")]
    ToolNotFound(String),

    #[error("Session error: {0}")]
    Session(String),

    #[error("Sandbox error: {0}")]
    Sandbox(String),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Schema build error: {0}")]
    Schema(String),

    #[error("Cancelled by user")]
    Cancelled,

    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, AgentError>;
