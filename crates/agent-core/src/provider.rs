use std::collections::HashMap;
use std::sync::RwLock;
use std::time::Instant;

use tracing::{debug, warn};

use crate::config::AppConfig;
use crate::error::AgentError;

/// A resolved provider ready for use (API key resolved from env or config).
#[derive(Debug, Clone)]
pub struct ResolvedProvider {
    pub name: String,
    pub api_base: String,
    pub model: String,
    pub api_key: Option<String>,
    pub priority: u32,
    pub timeout_secs: u64,
    pub max_retries: u32,
    pub roles: Vec<String>,
    pub max_tokens: u32,
    pub temperature: f32,
    pub top_p: f32,
}

/// Error classification for failover decisions.
#[derive(Debug, Clone)]
pub enum RequestError {
    /// Transient error — safe to retry with next provider (timeout, 5xx, network).
    Transient(String),
    /// Permanent error — stop trying (4xx, auth, bad request).
    Permanent(String),
}

/// Health tracking for a single provider.
#[derive(Debug, Default)]
struct ProviderHealth {
    consecutive_failures: u32,
    last_success: Option<Instant>,
    last_failure: Option<Instant>,
    total_requests: u64,
    total_failures: u64,
}

/// Multi-provider chain with health tracking and automatic failover.
pub struct ProviderChain {
    providers: Vec<ResolvedProvider>,
    health: RwLock<HashMap<String, ProviderHealth>>,
}

impl ProviderChain {
    /// Create a new provider chain from a list of resolved providers.
    pub fn new(providers: Vec<ResolvedProvider>) -> Self {
        let health = providers
            .iter()
            .map(|p| (p.name.clone(), ProviderHealth::default()))
            .collect();
        Self {
            providers,
            health: RwLock::new(health),
        }
    }

    /// Build a ProviderChain from AppConfig.
    ///
    /// If the `providers` array is non-empty, those entries are used.
    /// Otherwise the single `provider` config is wrapped (including any
    /// legacy `failover` endpoints) for backward compatibility.
    pub fn from_config(config: &AppConfig) -> Result<Self, AgentError> {
        let providers = if !config.providers.is_empty() {
            config
                .providers
                .iter()
                .map(|entry| {
                    let api_key = entry.api_key.clone().or_else(|| {
                        entry
                            .api_key_env
                            .as_ref()
                            .and_then(|env_var| std::env::var(env_var).ok())
                    });
                    ResolvedProvider {
                        name: entry.name.clone(),
                        api_base: entry.api_base.clone(),
                        model: entry.model.clone(),
                        api_key,
                        priority: entry.priority,
                        timeout_secs: entry.timeout_secs,
                        max_retries: entry.max_retries,
                        roles: entry.roles.clone(),
                        max_tokens: entry.max_tokens.unwrap_or(config.provider.max_tokens),
                        temperature: entry.temperature.unwrap_or(config.provider.temperature),
                        top_p: entry.top_p.unwrap_or(config.provider.top_p),
                    }
                })
                .collect()
        } else {
            // Backward compatibility: wrap [provider] + legacy failover entries.
            let mut chain = vec![ResolvedProvider {
                name: "default".to_string(),
                api_base: config.provider.api_base.clone(),
                model: config.provider.model.clone(),
                api_key: config.provider.api_key.clone(),
                priority: 1,
                timeout_secs: 30,
                max_retries: 2,
                roles: Vec::new(),
                max_tokens: config.provider.max_tokens,
                temperature: config.provider.temperature,
                top_p: config.provider.top_p,
            }];

            for (i, fo) in config.provider.failover.iter().enumerate() {
                chain.push(ResolvedProvider {
                    name: format!("failover-{}", i + 1),
                    api_base: fo.api_base.clone(),
                    model: fo
                        .model
                        .clone()
                        .unwrap_or_else(|| config.provider.model.clone()),
                    api_key: fo.api_key.clone(),
                    priority: (i + 2) as u32,
                    timeout_secs: 30,
                    max_retries: 2,
                    roles: Vec::new(),
                    max_tokens: config.provider.max_tokens,
                    temperature: config.provider.temperature,
                    top_p: config.provider.top_p,
                });
            }

            chain
        };

        if providers.is_empty() {
            return Err(AgentError::Config("No providers configured".into()));
        }

        Ok(Self::new(providers))
    }

    /// Select the best available provider for the given role.
    ///
    /// Filters by role (empty roles = matches any), excludes unhealthy providers,
    /// and returns the highest-priority (lowest number) candidate.
    pub fn select(&self, role: Option<&str>) -> Result<ResolvedProvider, AgentError> {
        let health = self
            .health
            .read()
            .map_err(|e| AgentError::Provider(format!("Health lock poisoned: {}", e)))?;

        let mut candidates: Vec<&ResolvedProvider> = self
            .providers
            .iter()
            .filter(|p| match role {
                Some(r) => p.roles.is_empty() || p.roles.iter().any(|pr| pr == r),
                None => true,
            })
            .filter(|p| {
                health
                    .get(&p.name)
                    .map(|h| h.consecutive_failures < p.max_retries)
                    .unwrap_or(true)
            })
            .collect();

        candidates.sort_by_key(|p| p.priority);

        candidates
            .first()
            .map(|p| (*p).clone())
            .ok_or_else(|| AgentError::Provider("All providers exhausted".into()))
    }

    /// Record a successful request for the named provider.
    pub fn record_success(&self, name: &str) {
        if let Ok(mut health) = self.health.write() {
            if let Some(h) = health.get_mut(name) {
                h.consecutive_failures = 0;
                h.last_success = Some(Instant::now());
                h.total_requests += 1;
            }
        }
    }

    /// Record a failed request for the named provider.
    pub fn record_failure(&self, name: &str) {
        if let Ok(mut health) = self.health.write() {
            if let Some(h) = health.get_mut(name) {
                h.consecutive_failures += 1;
                h.last_failure = Some(Instant::now());
                h.total_requests += 1;
                h.total_failures += 1;
            }
        }
    }

    /// Try a request with automatic failover across providers.
    ///
    /// The `make_request` closure receives an owned `ResolvedProvider` and returns
    /// the result classified as `Ok(T)` or `Err(RequestError)`.
    ///
    /// On `RequestError::Transient`, the next provider is tried.
    /// On `RequestError::Permanent`, the error is returned immediately.
    pub async fn request_with_failover<F, Fut, T>(
        &self,
        role: Option<&str>,
        make_request: F,
    ) -> Result<T, AgentError>
    where
        F: Fn(ResolvedProvider) -> Fut,
        Fut: std::future::Future<Output = Result<T, RequestError>>,
    {
        let candidates = {
            let health = self
                .health
                .read()
                .map_err(|e| AgentError::Provider(format!("Health lock poisoned: {}", e)))?;

            let mut candidates: Vec<ResolvedProvider> = self
                .providers
                .iter()
                .filter(|p| match role {
                    Some(r) => p.roles.is_empty() || p.roles.iter().any(|pr| pr == r),
                    None => true,
                })
                .filter(|p| {
                    health
                        .get(&p.name)
                        .map(|h| h.consecutive_failures < p.max_retries)
                        .unwrap_or(true)
                })
                .cloned()
                .collect();

            candidates.sort_by_key(|p| p.priority);
            candidates
        }; // Read lock released here before making requests.

        if candidates.is_empty() {
            return Err(AgentError::Provider("All providers exhausted".into()));
        }

        let mut errors = Vec::new();

        for provider in &candidates {
            debug!("Trying provider: {}", provider.name);

            match make_request(provider.clone()).await {
                Ok(result) => {
                    self.record_success(&provider.name);
                    return Ok(result);
                }
                Err(RequestError::Transient(msg)) => {
                    warn!("Provider {} transient error: {}", provider.name, msg);
                    self.record_failure(&provider.name);
                    errors.push(format!("{}: {}", provider.name, msg));
                }
                Err(RequestError::Permanent(msg)) => {
                    warn!("Provider {} permanent error: {}", provider.name, msg);
                    self.record_failure(&provider.name);
                    return Err(AgentError::Provider(format!(
                        "Provider {} permanent error: {}",
                        provider.name, msg
                    )));
                }
            }
        }

        Err(AgentError::Provider(format!(
            "All providers failed: {}",
            errors.join("; ")
        )))
    }

    /// Get the list of configured providers.
    pub fn providers(&self) -> &[ResolvedProvider] {
        &self.providers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_provider(
        name: &str,
        priority: u32,
        roles: Vec<&str>,
        max_retries: u32,
    ) -> ResolvedProvider {
        ResolvedProvider {
            name: name.to_string(),
            api_base: format!("http://{}.example.com/v1", name),
            model: format!("{}-model", name),
            api_key: Some(format!("{}-key", name)),
            priority,
            timeout_secs: 30,
            max_retries,
            roles: roles.into_iter().map(String::from).collect(),
            max_tokens: 4096,
            temperature: 0.7,
            top_p: 0.9,
        }
    }

    #[test]
    fn test_single_provider_backward_compat() {
        let config = AppConfig::default();
        let chain = ProviderChain::from_config(&config).unwrap();
        assert_eq!(chain.providers().len(), 1);
        assert_eq!(chain.providers()[0].name, "default");
        assert_eq!(chain.providers()[0].api_base, config.provider.api_base);
        assert_eq!(chain.providers()[0].model, config.provider.model);

        let selected = chain.select(None).unwrap();
        assert_eq!(selected.name, "default");
    }

    #[tokio::test]
    async fn test_failover_on_timeout() {
        let providers = vec![
            make_provider("fast", 1, vec![], 2),
            make_provider("backup", 2, vec![], 2),
        ];
        let chain = ProviderChain::new(providers);

        let result: Result<String, _> = chain
            .request_with_failover(None, |provider| async move {
                if provider.name == "fast" {
                    Err(RequestError::Transient("Request timed out".into()))
                } else {
                    Ok(format!("response from {}", provider.name))
                }
            })
            .await;

        assert_eq!(result.unwrap(), "response from backup");
    }

    #[tokio::test]
    async fn test_no_failover_on_auth_error() {
        let providers = vec![
            make_provider("primary", 1, vec![], 2),
            make_provider("backup", 2, vec![], 2),
        ];
        let chain = ProviderChain::new(providers);

        let result: Result<String, _> = chain
            .request_with_failover(None, |provider| async move {
                if provider.name == "primary" {
                    Err(RequestError::Permanent("invalid_api_key".into()))
                } else {
                    Ok(format!("response from {}", provider.name))
                }
            })
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("permanent"));
        assert!(err.contains("invalid_api_key"));
    }

    #[test]
    fn test_role_based_selection() {
        let providers = vec![
            make_provider("scout", 1, vec!["routine", "heartbeat"], 2),
            make_provider("claude", 2, vec!["complex", "creative"], 2),
        ];
        let chain = ProviderChain::new(providers);

        let routine = chain.select(Some("routine")).unwrap();
        assert_eq!(routine.name, "scout");

        let creative = chain.select(Some("creative")).unwrap();
        assert_eq!(creative.name, "claude");

        // No role filter — both match, lowest priority wins.
        let any = chain.select(None).unwrap();
        assert_eq!(any.name, "scout");
    }

    #[test]
    fn test_health_recovery_after_success() {
        let providers = vec![make_provider("test", 1, vec![], 3)];
        let chain = ProviderChain::new(providers);

        // Fail twice — still selectable (2 < 3).
        chain.record_failure("test");
        chain.record_failure("test");
        assert!(chain.select(None).is_ok());

        // Record success — resets consecutive failures.
        chain.record_success("test");

        // Fail once more — still selectable (1 < 3).
        chain.record_failure("test");
        assert!(chain.select(None).is_ok());
    }

    #[test]
    fn test_all_providers_exhausted_error() {
        let providers = vec![
            make_provider("a", 1, vec![], 1), // 1 failure exhausts
            make_provider("b", 2, vec![], 1),
        ];
        let chain = ProviderChain::new(providers);

        chain.record_failure("a");
        chain.record_failure("b");

        let result = chain.select(None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("exhausted"));
    }

    #[tokio::test]
    async fn test_all_providers_fail_transient() {
        let providers = vec![
            make_provider("a", 1, vec![], 2),
            make_provider("b", 2, vec![], 2),
        ];
        let chain = ProviderChain::new(providers);

        let result: Result<String, _> = chain
            .request_with_failover(None, |_provider| async move {
                Err(RequestError::Transient("server error".into()))
            })
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("All providers failed"));
    }

    #[test]
    fn test_role_no_match_returns_error() {
        let providers = vec![make_provider("scout", 1, vec!["routine"], 2)];
        let chain = ProviderChain::new(providers);

        let result = chain.select(Some("creative"));
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_roles_matches_any_role() {
        let providers = vec![make_provider("general", 1, vec![], 2)];
        let chain = ProviderChain::new(providers);

        assert!(chain.select(Some("routine")).is_ok());
        assert!(chain.select(Some("creative")).is_ok());
        assert!(chain.select(None).is_ok());
    }
}
