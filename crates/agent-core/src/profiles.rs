//! Named profile system for workspace-specific configuration.
//!
//! Profiles override top-level AppConfig fields (model, API base, system prompt,
//! max_tokens, temperature) so the user can switch contexts without editing the
//! config file.
//!
//! Ported from Claude-Code-CLI-Launcher workspace concepts.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::config::AppConfig;

/// A named profile that selectively overrides AppConfig fields.
///
/// Currently supported overrides: model, api_base, system_prompt, max_tokens,
/// temperature. Additional overrides (working_dir, tool allow/deny lists,
/// env_vars) may be added in future versions.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileConfig {
    /// Human-readable description.
    pub description: Option<String>,
    /// Override the model name.
    pub model: Option<String>,
    /// Override the API base URL.
    pub api_base: Option<String>,
    /// Override the system prompt.
    pub system_prompt: Option<String>,
    /// Maximum tokens override.
    pub max_tokens: Option<u32>,
    /// Temperature override.
    pub temperature: Option<f32>,
}

impl ProfileConfig {
    /// Apply this profile's overrides to a config, returning a new config.
    pub fn apply_to(&self, config: &AppConfig) -> AppConfig {
        let mut c = config.clone();

        if let Some(model) = &self.model {
            c.provider.model = model.clone();
        }
        if let Some(api_base) = &self.api_base {
            c.provider.api_base = api_base.clone();
        }
        if let Some(prompt) = &self.system_prompt {
            c.system_prompt = Some(prompt.clone());
        }
        if let Some(max_tokens) = self.max_tokens {
            c.provider.max_tokens = max_tokens;
        }
        if let Some(temperature) = self.temperature {
            c.provider.temperature = temperature;
        }

        c
    }
}

/// Resolve a named profile from the config's profile map.
///
/// Returns `None` if the profile name is not found.
pub fn resolve_profile<'a>(
    profiles: &'a HashMap<String, ProfileConfig>,
    name: &str,
) -> Option<&'a ProfileConfig> {
    profiles.get(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_apply_model_override() {
        let config = AppConfig::default();
        let profile = ProfileConfig {
            model: Some("llama3".into()),
            ..Default::default()
        };
        let applied = profile.apply_to(&config);
        assert_eq!(applied.provider.model, "llama3");
        // Other fields unchanged.
        assert_eq!(applied.provider.api_base, config.provider.api_base);
    }

    #[test]
    fn test_profile_apply_api_base_override() {
        let config = AppConfig::default();
        let profile = ProfileConfig {
            api_base: Some("https://api.groq.com/openai/v1".into()),
            ..Default::default()
        };
        let applied = profile.apply_to(&config);
        assert_eq!(applied.provider.api_base, "https://api.groq.com/openai/v1");
    }

    #[test]
    fn test_profile_apply_system_prompt() {
        let config = AppConfig::default();
        let profile = ProfileConfig {
            system_prompt: Some("You are a Rust expert.".into()),
            ..Default::default()
        };
        let applied = profile.apply_to(&config);
        assert_eq!(
            applied.system_prompt.as_deref(),
            Some("You are a Rust expert.")
        );
    }

    #[test]
    fn test_profile_apply_multiple_overrides() {
        let config = AppConfig::default();
        let profile = ProfileConfig {
            model: Some("gpt-4o".into()),
            max_tokens: Some(8192),
            temperature: Some(0.3),
            ..Default::default()
        };
        let applied = profile.apply_to(&config);
        assert_eq!(applied.provider.model, "gpt-4o");
        assert_eq!(applied.provider.max_tokens, 8192);
        assert_eq!(applied.provider.temperature, 0.3);
    }

    #[test]
    fn test_profile_no_overrides_is_identity() {
        let config = AppConfig::default();
        let profile = ProfileConfig::default();
        let applied = profile.apply_to(&config);
        assert_eq!(applied.provider.model, config.provider.model);
        assert_eq!(applied.provider.api_base, config.provider.api_base);
        assert_eq!(applied.provider.max_tokens, config.provider.max_tokens);
        assert_eq!(applied.system_prompt, config.system_prompt);
    }

    #[test]
    fn test_resolve_profile_found() {
        let mut profiles = HashMap::new();
        profiles.insert(
            "work".to_string(),
            ProfileConfig {
                model: Some("claude-opus-4-20250514".into()),
                ..Default::default()
            },
        );
        let result = resolve_profile(&profiles, "work");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().model.as_deref(),
            Some("claude-opus-4-20250514")
        );
    }

    #[test]
    fn test_resolve_profile_not_found() {
        let profiles: HashMap<String, ProfileConfig> = HashMap::new();
        assert!(resolve_profile(&profiles, "nonexistent").is_none());
    }

    #[test]
    fn test_profile_serialization() {
        let profile = ProfileConfig {
            description: Some("Dev profile".into()),
            model: Some("llama3".into()),
            temperature: Some(0.7),
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&profile).unwrap();
        let parsed: ProfileConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.description.as_deref(), Some("Dev profile"));
        assert_eq!(parsed.model.as_deref(), Some("llama3"));
        assert_eq!(parsed.temperature, Some(0.7));
    }
}
