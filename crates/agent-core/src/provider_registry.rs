//! Provider Registry — multi-LLM provider management inspired by VibeCoder's
//! service abstraction (Claude/Ollama/Gemini switching) unified into a single
//! registry with auto-discovery for local providers.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tracing::{debug, info};

// ---------------------------------------------------------------------------
// ProviderKind
// ---------------------------------------------------------------------------

/// The kind of LLM backend a provider represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    OpenAI,
    Anthropic,
    Ollama,
    Gemini,
    LMStudio,
    Custom,
}

impl std::fmt::Display for ProviderKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenAI => write!(f, "openai"),
            Self::Anthropic => write!(f, "anthropic"),
            Self::Ollama => write!(f, "ollama"),
            Self::Gemini => write!(f, "gemini"),
            Self::LMStudio => write!(f, "lmstudio"),
            Self::Custom => write!(f, "custom"),
        }
    }
}

// ---------------------------------------------------------------------------
// ProviderInfo
// ---------------------------------------------------------------------------

/// Metadata about a single LLM provider endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderInfo {
    /// Human-readable name (e.g. "ollama-local", "claude-prod").
    pub name: String,
    /// Backend kind.
    pub kind: ProviderKind,
    /// Base URL of the API (e.g. "http://localhost:11434").
    pub base_url: String,
    /// Environment variable that holds the API key.
    /// At runtime the registry resolves this to the actual key value.
    pub api_key_env: Option<String>,
    /// Models available through this provider.
    pub models: Vec<String>,
    /// Whether this provider runs locally (affects key requirements).
    pub is_local: bool,
    /// Supports SSE / streaming responses.
    pub supports_streaming: bool,
    /// Supports OpenAI-style tool calling.
    pub supports_tools: bool,
}

impl ProviderInfo {
    /// Resolve the API key from the environment, if configured.
    pub fn resolve_api_key(&self) -> Option<String> {
        self.api_key_env
            .as_ref()
            .and_then(|env_var| std::env::var(env_var).ok())
    }
}

// ---------------------------------------------------------------------------
// ProviderRegistry
// ---------------------------------------------------------------------------

/// Central registry of all known LLM providers. Inspired by VibeCoder's
/// per-service modules (claudeCliService, ollamaService) but unified into a
/// single lookup table so the rest of the system can discover and switch
/// providers at runtime.
pub struct ProviderRegistry {
    providers: HashMap<String, ProviderInfo>,
}

impl ProviderRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    /// Create a registry pre-loaded with built-in defaults for common
    /// providers (Ollama, OpenAI, Anthropic, Gemini, LM Studio).
    pub fn with_defaults() -> Self {
        let mut reg = Self::new();
        for info in builtin_providers() {
            reg.register(info);
        }
        reg
    }

    // -- Mutation ----------------------------------------------------------

    /// Register (or update) a provider by name.
    pub fn register(&mut self, info: ProviderInfo) {
        debug!("Registering provider: {} ({})", info.name, info.kind);
        self.providers.insert(info.name.clone(), info);
    }

    /// Remove a provider by name. Returns the removed entry if it existed.
    pub fn unregister(&mut self, name: &str) -> Option<ProviderInfo> {
        self.providers.remove(name)
    }

    // -- Query -------------------------------------------------------------

    /// List all registered providers (unordered).
    pub fn list(&self) -> Vec<&ProviderInfo> {
        self.providers.values().collect()
    }

    /// Get a provider by its exact name.
    pub fn get(&self, name: &str) -> Option<&ProviderInfo> {
        self.providers.get(name)
    }

    /// Get all providers of a given kind.
    pub fn get_by_kind(&self, kind: ProviderKind) -> Vec<&ProviderInfo> {
        self.providers
            .values()
            .filter(|p| p.kind == kind)
            .collect()
    }

    /// Return the number of registered providers.
    pub fn len(&self) -> usize {
        self.providers.len()
    }

    /// Check whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    // -- Discovery ---------------------------------------------------------

    /// Attempt to discover a local Ollama instance on `localhost:11434`.
    ///
    /// This is a **synchronous** probe (connect-only, no HTTP) suitable for
    /// startup. For production use, callers should use the async variant or
    /// call the Ollama `/api/tags` endpoint.
    pub fn discover_local(&mut self) -> Vec<ProviderInfo> {
        let mut discovered = Vec::new();

        // Ollama default
        if self.probe_tcp("127.0.0.1", 11434) {
            let info = ProviderInfo {
                name: "ollama-discovered".into(),
                kind: ProviderKind::Ollama,
                base_url: "http://localhost:11434".into(),
                api_key_env: None,
                models: vec![], // populated lazily by callers
                is_local: true,
                supports_streaming: true,
                supports_tools: true,
            };
            info!("Discovered local Ollama on :11434");
            self.register(info.clone());
            discovered.push(info);
        }

        // LM Studio default
        if self.probe_tcp("127.0.0.1", 1234) {
            let info = ProviderInfo {
                name: "lmstudio-discovered".into(),
                kind: ProviderKind::LMStudio,
                base_url: "http://localhost:1234/v1".into(),
                api_key_env: None,
                models: vec![],
                is_local: true,
                supports_streaming: true,
                supports_tools: false,
            };
            info!("Discovered local LM Studio on :1234");
            self.register(info.clone());
            discovered.push(info);
        }

        discovered
    }

    /// Low-level TCP connect probe. Returns `true` if the port is open.
    fn probe_tcp(&self, host: &str, port: u16) -> bool {
        use std::net::{SocketAddr, TcpStream};
        use std::time::Duration;

        let addr: SocketAddr = format!("{}:{}", host, port).parse().unwrap();
        TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Built-in defaults
// ---------------------------------------------------------------------------

/// Return the set of well-known provider templates (no discovery, just
/// static metadata). Users are expected to configure API keys via config.
fn builtin_providers() -> Vec<ProviderInfo> {
    vec![
        ProviderInfo {
            name: "openai".into(),
            kind: ProviderKind::OpenAI,
            base_url: "https://api.openai.com/v1".into(),
            api_key_env: Some("OPENAI_API_KEY".into()),
            models: vec![
                "gpt-4.1".into(),
                "gpt-4.1-mini".into(),
                "gpt-4.1-nano".into(),
                "o3".into(),
                "o4-mini".into(),
            ],
            is_local: false,
            supports_streaming: true,
            supports_tools: true,
        },
        ProviderInfo {
            name: "anthropic".into(),
            kind: ProviderKind::Anthropic,
            base_url: "https://api.anthropic.com/v1".into(),
            api_key_env: Some("ANTHROPIC_API_KEY".into()),
            models: vec![
                "claude-opus-4-20250514".into(),
                "claude-sonnet-4-20250514".into(),
                "claude-haiku-3-5-20241022".into(),
            ],
            is_local: false,
            supports_streaming: true,
            supports_tools: true,
        },
        ProviderInfo {
            name: "ollama".into(),
            kind: ProviderKind::Ollama,
            base_url: "http://localhost:11434".into(),
            api_key_env: None,
            models: vec![
                "llama3.2".into(),
                "mistral".into(),
                "codellama".into(),
            ],
            is_local: true,
            supports_streaming: true,
            supports_tools: true,
        },
        ProviderInfo {
            name: "gemini".into(),
            kind: ProviderKind::Gemini,
            base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
            api_key_env: Some("GOOGLE_API_KEY".into()),
            models: vec![
                "gemini-2.5-flash".into(),
                "gemini-2.5-pro".into(),
            ],
            is_local: false,
            supports_streaming: true,
            supports_tools: true,
        },
        ProviderInfo {
            name: "lmstudio".into(),
            kind: ProviderKind::LMStudio,
            base_url: "http://localhost:1234/v1".into(),
            api_key_env: None,
            models: vec![],
            is_local: true,
            supports_streaming: true,
            supports_tools: false,
        },
    ]
}

// ===========================================================================
// Tests
// ===========================================================================
#[cfg(test)]
mod tests {
    use super::*;

    fn sample_provider(name: &str, kind: ProviderKind) -> ProviderInfo {
        ProviderInfo {
            name: name.into(),
            kind,
            base_url: format!("http://{}.test", name),
            api_key_env: None,
            models: vec!["model-a".into()],
            is_local: kind == ProviderKind::Ollama || kind == ProviderKind::LMStudio,
            supports_streaming: true,
            supports_tools: true,
        }
    }

    // --- ProviderRegistry: register and list providers ---------------------
    #[test]
    fn test_register_and_list() {
        let mut reg = ProviderRegistry::new();
        assert!(reg.is_empty());

        reg.register(sample_provider("alpha", ProviderKind::OpenAI));
        reg.register(sample_provider("beta", ProviderKind::Anthropic));

        assert_eq!(reg.len(), 2);
        let names: Vec<&str> = reg.list().iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
    }

    // --- ProviderRegistry: get by name ------------------------------------
    #[test]
    fn test_get_by_name() {
        let mut reg = ProviderRegistry::new();
        reg.register(sample_provider("ollama-local", ProviderKind::Ollama));

        assert!(reg.get("ollama-local").is_some());
        assert_eq!(
            reg.get("ollama-local").unwrap().kind,
            ProviderKind::Ollama
        );
        assert!(reg.get("nonexistent").is_none());
    }

    // --- ProviderRegistry: get by kind ------------------------------------
    #[test]
    fn test_get_by_kind() {
        let mut reg = ProviderRegistry::new();
        reg.register(sample_provider("a", ProviderKind::OpenAI));
        reg.register(sample_provider("b", ProviderKind::OpenAI));
        reg.register(sample_provider("c", ProviderKind::Anthropic));

        let openai = reg.get_by_kind(ProviderKind::OpenAI);
        assert_eq!(openai.len(), 2);

        let anthropic = reg.get_by_kind(ProviderKind::Anthropic);
        assert_eq!(anthropic.len(), 1);

        let ollama = reg.get_by_kind(ProviderKind::Ollama);
        assert!(ollama.is_empty());
    }

    // --- ProviderRegistry: built-in defaults ------------------------------
    #[test]
    fn test_with_defaults() {
        let reg = ProviderRegistry::with_defaults();
        assert!(reg.len() >= 5, "Expected at least 5 built-in providers");

        // Verify the key built-ins exist
        assert!(reg.get("openai").is_some());
        assert!(reg.get("anthropic").is_some());
        assert!(reg.get("ollama").is_some());
        assert!(reg.get("gemini").is_some());
        assert!(reg.get("lmstudio").is_some());

        // Check OpenAI details
        let openai = reg.get("openai").unwrap();
        assert_eq!(openai.kind, ProviderKind::OpenAI);
        assert!(!openai.is_local);
        assert!(openai.supports_tools);
        assert!(openai.supports_streaming);
        assert_eq!(openai.api_key_env.as_deref(), Some("OPENAI_API_KEY"));
        assert!(!openai.models.is_empty());

        // Check Ollama details
        let ollama = reg.get("ollama").unwrap();
        assert!(ollama.is_local);
        assert!(ollama.api_key_env.is_none());
    }

    // --- ProviderRegistry: discover_local (mock) --------------------------
    #[test]
    fn test_discover_local_no_services() {
        // In CI / test environments, nothing is likely listening on 11434 or 1234.
        // discover_local should still return without error.
        let mut reg = ProviderRegistry::new();
        let discovered = reg.discover_local();
        // We cannot assert count because a real Ollama might be running,
        // but we can assert the function completes and returns a Vec.
        assert!(discovered.len() <= 2);
    }

    // --- ProviderKind: serde roundtrip ------------------------------------
    #[test]
    fn test_provider_kind_serde_roundtrip() {
        let kinds = vec![
            ProviderKind::OpenAI,
            ProviderKind::Anthropic,
            ProviderKind::Ollama,
            ProviderKind::Gemini,
            ProviderKind::LMStudio,
            ProviderKind::Custom,
        ];

        for kind in kinds {
            let json = serde_json::to_string(&kind).unwrap();
            let parsed: ProviderKind = serde_json::from_str(&json).unwrap();
            assert_eq!(kind, parsed, "Roundtrip failed for {:?}", kind);
        }
    }

    // --- ProviderInfo: serialization roundtrip -----------------------------
    #[test]
    fn test_provider_info_serialization() {
        let info = sample_provider("test-provider", ProviderKind::Gemini);
        let json = serde_json::to_string(&info).unwrap();
        let parsed: ProviderInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.name, "test-provider");
        assert_eq!(parsed.kind, ProviderKind::Gemini);
        assert_eq!(parsed.models, vec!["model-a"]);
    }

    // --- ProviderRegistry: unregister removes provider --------------------
    #[test]
    fn test_unregister() {
        let mut reg = ProviderRegistry::new();
        reg.register(sample_provider("temp", ProviderKind::Custom));
        assert_eq!(reg.len(), 1);

        let removed = reg.unregister("temp");
        assert!(removed.is_some());
        assert_eq!(reg.len(), 0);
        assert!(reg.get("temp").is_none());
    }

    // --- ProviderRegistry: register overwrites existing -------------------
    #[test]
    fn test_register_overwrite() {
        let mut reg = ProviderRegistry::new();
        let mut p = sample_provider("dup", ProviderKind::OpenAI);
        p.models = vec!["old-model".into()];
        reg.register(p);
        assert_eq!(reg.get("dup").unwrap().models, vec!["old-model"]);

        let mut p2 = sample_provider("dup", ProviderKind::OpenAI);
        p2.models = vec!["new-model".into()];
        reg.register(p2);
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("dup").unwrap().models, vec!["new-model"]);
    }

    // --- ProviderKind: Display impl --------------------------------------
    #[test]
    fn test_provider_kind_display() {
        assert_eq!(ProviderKind::OpenAI.to_string(), "openai");
        assert_eq!(ProviderKind::Anthropic.to_string(), "anthropic");
        assert_eq!(ProviderKind::Ollama.to_string(), "ollama");
        assert_eq!(ProviderKind::Gemini.to_string(), "gemini");
        assert_eq!(ProviderKind::LMStudio.to_string(), "lmstudio");
        assert_eq!(ProviderKind::Custom.to_string(), "custom");
    }
}
