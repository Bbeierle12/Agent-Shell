use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Top-level application configuration, loaded from TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub provider: ProviderConfig,
    pub sandbox: SandboxConfig,
    pub rag: RagConfig,
    pub server: ServerConfig,
    pub session: SessionConfig,
    pub system_prompt: Option<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            provider: ProviderConfig::default(),
            sandbox: SandboxConfig::default(),
            rag: RagConfig::default(),
            server: ServerConfig::default(),
            session: SessionConfig::default(),
            system_prompt: Some(
                "You are a helpful AI assistant with access to tools. \
                 Use tools when appropriate to help the user. \
                 Think step by step before acting."
                    .into(),
            ),
        }
    }
}

impl AppConfig {
    /// Load configuration from default path (~/.config/agent-shell/config.toml),
    /// falling back to defaults if the file doesn't exist.
    pub fn load() -> anyhow::Result<Self> {
        let path = Self::default_path();
        if path.exists() {
            Self::load_from(&path)
        } else {
            Ok(Self::default())
        }
    }

    /// Load configuration from a specific path.
    pub fn load_from(path: &Path) -> anyhow::Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&contents)?;
        Ok(config)
    }

    /// Write current configuration to the default path.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::default_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let contents = toml::to_string_pretty(self)?;
        std::fs::write(&path, contents)?;
        Ok(())
    }

    /// Default config file path.
    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("agent-shell")
            .join("config.toml")
    }

    /// Data directory for sessions, indexes, etc.
    pub fn data_dir() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("agent-shell")
    }
}

/// LLM provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderConfig {
    /// Base URL for the OpenAI-compatible API.
    pub api_base: String,
    /// Model name (e.g. "glm-4.7-swift", "llama3", etc.).
    pub model: String,
    /// Optional API key.
    pub api_key: Option<String>,
    /// Maximum tokens to generate.
    pub max_tokens: u32,
    /// Sampling temperature.
    pub temperature: f32,
    /// Top-p sampling.
    pub top_p: f32,
    /// Failover endpoints — tried in order if primary fails.
    pub failover: Vec<FailoverEndpoint>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            api_base: "http://localhost:11434/v1".into(),
            model: "glm-4.7-swift".into(),
            api_key: None,
            max_tokens: 4096,
            temperature: 0.7,
            top_p: 0.9,
            failover: Vec::new(),
        }
    }
}

/// A failover endpoint for provider rotation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailoverEndpoint {
    pub api_base: String,
    pub model: Option<String>,
    pub api_key: Option<String>,
}

/// Sandbox configuration for code execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SandboxConfig {
    /// Sandbox mode: "docker" for isolated containers, "unsafe" for direct execution.
    pub mode: SandboxMode,
    /// Docker image to use for sandboxed execution.
    pub docker_image: String,
    /// Execution timeout in seconds.
    pub timeout_secs: u64,
    /// Maximum memory for Docker containers (in bytes).
    pub memory_limit: Option<u64>,
    /// Working directory inside the sandbox.
    pub work_dir: String,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            mode: SandboxMode::Unsafe,
            docker_image: "python:3.12-slim".into(),
            timeout_secs: 30,
            memory_limit: Some(512 * 1024 * 1024), // 512MB
            work_dir: "/workspace".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SandboxMode {
    Docker,
    Unsafe,
}

/// RAG / vector store configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RagConfig {
    /// Qdrant server URL.
    pub qdrant_url: String,
    /// Collection name for document chunks.
    pub collection_name: String,
    /// Embedding model name (for fastembed or the /v1/embeddings endpoint).
    pub embedding_model: String,
    /// Use local fastembed instead of the API endpoint for embeddings.
    pub use_local_embeddings: bool,
    /// Chunk size in characters.
    pub chunk_size: usize,
    /// Chunk overlap in characters.
    pub chunk_overlap: usize,
    /// Number of retrieval results.
    pub top_k: usize,
}

impl Default for RagConfig {
    fn default() -> Self {
        Self {
            qdrant_url: "http://localhost:6334".into(),
            collection_name: "agent-shell-docs".into(),
            embedding_model: "BAAI/bge-small-en-v1.5".into(),
            use_local_embeddings: true,
            chunk_size: 1000,
            chunk_overlap: 200,
            top_k: 5,
        }
    }
}

/// HTTP server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Bind address.
    pub host: String,
    /// Port.
    pub port: u16,
    /// Bearer token for authentication (None = no auth).
    pub auth_token: Option<String>,
    /// Enable CORS.
    pub cors: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            port: 8080,
            auth_token: None,
            cors: true,
        }
    }
}

/// Session persistence configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionConfig {
    /// Directory for persisting sessions.
    pub history_dir: Option<PathBuf>,
    /// Maximum messages to keep in history for context window.
    pub max_history: usize,
    /// Automatically save sessions on each message.
    pub auto_save: bool,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            history_dir: None, // resolved at runtime to data_dir/sessions
            max_history: 100,
            auto_save: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_serializes() {
        let config = AppConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        assert!(toml_str.contains("glm-4.7-swift"));
        assert!(toml_str.contains("localhost"));
    }

    #[test]
    fn test_config_roundtrip() {
        let config = AppConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: AppConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.provider.model, config.provider.model);
        assert_eq!(parsed.provider.max_tokens, config.provider.max_tokens);
    }
}
