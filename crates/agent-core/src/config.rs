use crate::profiles::ProfileConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Top-level application configuration, loaded from TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub provider: ProviderConfig,
    /// Multi-provider chain (opt-in). When non-empty, replaces the single `provider`.
    pub providers: Vec<ProviderEntry>,
    /// Scheduled tasks (opt-in).
    pub schedules: Vec<ScheduleConfig>,
    /// Named profiles for workspace-specific overrides.
    #[serde(default)]
    pub profiles: HashMap<String, ProfileConfig>,
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
            providers: Vec::new(),
            schedules: Vec::new(),
            profiles: HashMap::new(),
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

/// A failover endpoint for provider rotation (legacy format).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailoverEndpoint {
    pub api_base: String,
    pub model: Option<String>,
    pub api_key: Option<String>,
}

/// A provider entry in the `[[providers]]` multi-provider chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEntry {
    pub name: String,
    pub api_base: String,
    pub model: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default = "default_priority")]
    pub priority: u32,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
}

fn default_priority() -> u32 {
    1
}
fn default_timeout_secs() -> u64 {
    30
}
fn default_max_retries() -> u32 {
    2
}

/// A scheduled task entry in the `[[schedules]]` array.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleConfig {
    pub name: String,
    /// Cron expression (5-field standard or 7-field extended).
    pub cron: String,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default = "default_schedule_task")]
    pub task: ScheduleTaskType,
    /// Skill to load for heartbeat tasks (Phase 2).
    #[serde(default)]
    pub skill: Option<String>,
    /// Prompt text for prompt-type tasks.
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleTaskType {
    Heartbeat,
    Prompt,
    Custom,
}

fn default_schedule_task() -> ScheduleTaskType {
    ScheduleTaskType::Prompt
}
fn default_enabled() -> bool {
    true
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
    /// If set, file tools are restricted to paths under this directory.
    pub workspace_root: Option<PathBuf>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            mode: SandboxMode::Docker,
            docker_image: "python:3.12-slim".into(),
            timeout_secs: 30,
            memory_limit: Some(512 * 1024 * 1024), // 512MB
            work_dir: "/workspace".into(),
            workspace_root: None,
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

    #[test]
    fn test_config_without_workspace_root() {
        // A TOML config that does not mention workspace_root should
        // deserialize with workspace_root = None (backward compat).
        let toml_str = r#"
[sandbox]
mode = "docker"
timeout_secs = 30
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert!(config.sandbox.workspace_root.is_none());
    }

    #[test]
    fn test_default_sandbox_mode_is_docker() {
        assert_eq!(SandboxConfig::default().mode, SandboxMode::Docker);
    }

    #[test]
    fn test_providers_array_deserializes() {
        let toml_str = r#"
[[providers]]
name = "scout"
api_base = "https://api.groq.com/openai/v1"
model = "llama-4-scout"
api_key_env = "GROQ_API_KEY"
priority = 1
roles = ["routine"]

[[providers]]
name = "claude"
api_base = "https://api.anthropic.com/v1"
model = "claude-sonnet-4-5-20250929"
api_key_env = "ANTHROPIC_API_KEY"
priority = 2
timeout_secs = 60
max_retries = 1
roles = ["complex", "creative"]
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.providers.len(), 2);
        assert_eq!(config.providers[0].name, "scout");
        assert_eq!(config.providers[1].name, "claude");
        assert_eq!(config.providers[1].timeout_secs, 60);
        assert_eq!(config.providers[1].max_retries, 1);
        assert_eq!(config.providers[0].roles, vec!["routine"]);
    }

    #[test]
    fn test_schedules_array_deserializes() {
        let toml_str = r#"
[[schedules]]
name = "heartbeat"
cron = "*/30 * * * *"
workspace = "raisinbolt"
task = "heartbeat"
skill = "moltbook"

[[schedules]]
name = "consolidate"
cron = "0 */4 * * *"
task = "prompt"
prompt = "Consolidate memory."
enabled = false
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.schedules.len(), 2);
        assert_eq!(config.schedules[0].name, "heartbeat");
        assert_eq!(config.schedules[0].task, ScheduleTaskType::Heartbeat);
        assert_eq!(config.schedules[0].skill.as_deref(), Some("moltbook"));
        assert!(config.schedules[0].enabled);
        assert!(!config.schedules[1].enabled);
    }

    #[test]
    fn test_empty_schedules_backward_compat() {
        let config = AppConfig::default();
        assert!(config.schedules.is_empty());
    }

    #[test]
    fn test_profiles_deserialize() {
        let toml_str = r#"
[profiles.work]
description = "Work profile"
model = "claude-opus-4-20250514"
api_base = "https://api.anthropic.com/v1"
system_prompt = "You are a senior engineer."
max_tokens = 8192
temperature = 0.2

[profiles.personal]
model = "llama3"
working_dir = "/home/user/personal"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.profiles.len(), 2);
        let work = &config.profiles["work"];
        assert_eq!(work.model.as_deref(), Some("claude-opus-4-20250514"));
        assert_eq!(work.max_tokens, Some(8192));
        let personal = &config.profiles["personal"];
        assert_eq!(personal.model.as_deref(), Some("llama3"));
        assert!(personal.api_base.is_none());
    }

    #[test]
    fn test_empty_profiles_backward_compat() {
        let config = AppConfig::default();
        assert!(config.profiles.is_empty());
    }

    #[test]
    fn test_empty_providers_uses_single_provider() {
        let toml_str = r#"
[provider]
api_base = "http://localhost:11434/v1"
model = "test-model"
"#;
        let config: AppConfig = toml::from_str(toml_str).unwrap();
        assert!(config.providers.is_empty());
        assert_eq!(config.provider.model, "test-model");
    }
}
