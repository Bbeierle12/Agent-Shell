//! Plugin system for agent-shell.
//!
//! Provides a `Plugin` trait with lifecycle hooks and a `PluginRegistry`
//! for registering, querying, and controlling plugins at runtime.
//! Ported from netsec-core with agent-specific categories.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

// ── Category & Status ──────────────────────────────────────────────────

/// Agent-specific plugin categories.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PluginCategory {
    /// Adds a new tool to the agent loop (shell, file_ops, web_fetch, …).
    Tool,
    /// Adds or wraps an LLM provider endpoint.
    Provider,
    /// Supplies skill files / prompt templates.
    Skill,
    /// General-purpose extension (logging, analytics, integrations, …).
    Extension,
}

/// Plugin operational status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PluginStatus {
    Available,
    Unavailable,
    Running,
    Error,
}

// ── Plugin Trait ────────────────────────────────────────────────────────

/// Metadata describing a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub category: PluginCategory,
    pub status: PluginStatus,
    pub description: String,
}

/// Trait that all plugins must implement.
///
/// Provides lifecycle hooks (start/stop), health checks, and metadata.
/// Methods are synchronous to keep the trait dyn-compatible; plugins that
/// need async initialization should handle that internally.
pub trait Plugin: Send + Sync {
    /// Return metadata about this plugin.
    fn info(&self) -> PluginInfo;

    /// Perform a health check. Returns the current operational status.
    fn health_check(&self) -> PluginStatus;

    /// Start the plugin (initialize resources, begin background work, etc.).
    fn start(&mut self) -> Result<(), String>;

    /// Stop the plugin (release resources, shut down background work, etc.).
    fn stop(&mut self) -> Result<(), String>;
}

// ── Plugin Key ─────────────────────────────────────────────────────────

/// Unique key for a registered plugin (category + name).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PluginKey {
    pub category: PluginCategory,
    pub name: String,
}

impl PluginKey {
    pub fn new(category: PluginCategory, name: impl Into<String>) -> Self {
        Self {
            category,
            name: name.into(),
        }
    }
}

impl fmt::Display for PluginKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}/{}", self.category, self.name)
    }
}

// ── Registry ───────────────────────────────────────────────────────────

/// Central registry for all plugins.
pub struct PluginRegistry {
    plugins: HashMap<PluginKey, Box<dyn Plugin>>,
}

impl PluginRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    /// Register a plugin. Returns an error if a plugin with the same key already exists.
    pub fn register(&mut self, plugin: Box<dyn Plugin>) -> Result<(), String> {
        let info = plugin.info();
        let key = PluginKey::new(info.category.clone(), &info.name);
        if self.plugins.contains_key(&key) {
            return Err(format!("plugin already registered: {key}"));
        }
        tracing::info!("Registered plugin: {key}");
        self.plugins.insert(key, plugin);
        Ok(())
    }

    /// Unregister a plugin by key. Returns an error if not found.
    pub fn unregister(&mut self, key: &PluginKey) -> Result<(), String> {
        self.plugins
            .remove(key)
            .map(|_| {
                tracing::info!("Unregistered plugin: {key}");
            })
            .ok_or_else(|| format!("plugin not found: {key}"))
    }

    /// Get info for a specific plugin.
    pub fn get_info(&self, key: &PluginKey) -> Option<PluginInfo> {
        self.plugins.get(key).map(|p| p.info())
    }

    /// List info for all registered plugins.
    pub fn list(&self) -> Vec<PluginInfo> {
        self.plugins.values().map(|p| p.info()).collect()
    }

    /// List info for all plugins in a given category.
    pub fn list_by_category(&self, category: &PluginCategory) -> Vec<PluginInfo> {
        self.plugins
            .iter()
            .filter(|(k, _)| &k.category == category)
            .map(|(_, p)| p.info())
            .collect()
    }

    /// Run health checks on all plugins and return their statuses.
    pub fn health_check_all(&self) -> Vec<(PluginKey, PluginStatus)> {
        self.plugins
            .iter()
            .map(|(key, plugin)| (key.clone(), plugin.health_check()))
            .collect()
    }

    /// Start all plugins. Returns errors for any that fail (does not stop on first error).
    pub fn start_all(&mut self) -> Vec<(PluginKey, Result<(), String>)> {
        let keys: Vec<PluginKey> = self.plugins.keys().cloned().collect();
        let mut results = Vec::new();
        for key in keys {
            if let Some(plugin) = self.plugins.get_mut(&key) {
                let result = plugin.start();
                if let Err(ref e) = result {
                    tracing::warn!("Plugin {key} failed to start: {e}");
                }
                results.push((key, result));
            }
        }
        results
    }

    /// Stop all plugins. Returns errors for any that fail (does not stop on first error).
    pub fn stop_all(&mut self) -> Vec<(PluginKey, Result<(), String>)> {
        let keys: Vec<PluginKey> = self.plugins.keys().cloned().collect();
        let mut results = Vec::new();
        for key in keys {
            if let Some(plugin) = self.plugins.get_mut(&key) {
                let result = plugin.stop();
                if let Err(ref e) = result {
                    tracing::warn!("Plugin {key} failed to stop: {e}");
                }
                results.push((key, result));
            }
        }
        results
    }

    /// Return the total number of registered plugins.
    pub fn count(&self) -> usize {
        self.plugins.len()
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    /// A mock plugin for testing the registry.
    struct MockPlugin {
        name: String,
        category: PluginCategory,
        started: Arc<AtomicBool>,
        stopped: Arc<AtomicBool>,
        health: PluginStatus,
    }

    impl MockPlugin {
        fn new(name: &str, category: PluginCategory) -> Self {
            Self {
                name: name.to_string(),
                category,
                started: Arc::new(AtomicBool::new(false)),
                stopped: Arc::new(AtomicBool::new(false)),
                health: PluginStatus::Available,
            }
        }

        fn with_health(mut self, status: PluginStatus) -> Self {
            self.health = status;
            self
        }
    }

    impl Plugin for MockPlugin {
        fn info(&self) -> PluginInfo {
            PluginInfo {
                name: self.name.clone(),
                version: "1.0.0".to_string(),
                category: self.category.clone(),
                status: if self.started.load(Ordering::Relaxed) {
                    PluginStatus::Running
                } else {
                    PluginStatus::Available
                },
                description: format!("Mock {} plugin", self.name),
            }
        }

        fn health_check(&self) -> PluginStatus {
            self.health.clone()
        }

        fn start(&mut self) -> Result<(), String> {
            self.started.store(true, Ordering::Relaxed);
            Ok(())
        }

        fn stop(&mut self) -> Result<(), String> {
            self.stopped.store(true, Ordering::Relaxed);
            self.started.store(false, Ordering::Relaxed);
            Ok(())
        }
    }

    /// A mock plugin that fails on start/stop.
    struct FailingPlugin;

    impl Plugin for FailingPlugin {
        fn info(&self) -> PluginInfo {
            PluginInfo {
                name: "failing".to_string(),
                version: "0.0.1".to_string(),
                category: PluginCategory::Tool,
                status: PluginStatus::Error,
                description: "Always fails".to_string(),
            }
        }

        fn health_check(&self) -> PluginStatus {
            PluginStatus::Error
        }

        fn start(&mut self) -> Result<(), String> {
            Err("start failed".to_string())
        }

        fn stop(&mut self) -> Result<(), String> {
            Err("stop failed".to_string())
        }
    }

    #[test]
    fn test_register_and_count() {
        let mut registry = PluginRegistry::new();
        assert_eq!(registry.count(), 0);

        let plugin = MockPlugin::new("shell_exec", PluginCategory::Tool);
        registry.register(Box::new(plugin)).unwrap();
        assert_eq!(registry.count(), 1);
    }

    #[test]
    fn test_register_duplicate_fails() {
        let mut registry = PluginRegistry::new();

        let p1 = MockPlugin::new("shell_exec", PluginCategory::Tool);
        registry.register(Box::new(p1)).unwrap();

        let p2 = MockPlugin::new("shell_exec", PluginCategory::Tool);
        let err = registry.register(Box::new(p2)).unwrap_err();
        assert!(err.contains("already registered"));
    }

    #[test]
    fn test_same_name_different_category() {
        let mut registry = PluginRegistry::new();

        let p1 = MockPlugin::new("openai", PluginCategory::Tool);
        let p2 = MockPlugin::new("openai", PluginCategory::Provider);
        registry.register(Box::new(p1)).unwrap();
        registry.register(Box::new(p2)).unwrap();
        assert_eq!(registry.count(), 2);
    }

    #[test]
    fn test_unregister() {
        let mut registry = PluginRegistry::new();

        let plugin = MockPlugin::new("web_fetch", PluginCategory::Tool);
        registry.register(Box::new(plugin)).unwrap();
        assert_eq!(registry.count(), 1);

        let key = PluginKey::new(PluginCategory::Tool, "web_fetch");
        registry.unregister(&key).unwrap();
        assert_eq!(registry.count(), 0);
    }

    #[test]
    fn test_unregister_not_found() {
        let mut registry = PluginRegistry::new();
        let key = PluginKey::new(PluginCategory::Tool, "nonexistent");
        let err = registry.unregister(&key).unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn test_get_info() {
        let mut registry = PluginRegistry::new();
        let plugin = MockPlugin::new("groq", PluginCategory::Provider);
        registry.register(Box::new(plugin)).unwrap();

        let key = PluginKey::new(PluginCategory::Provider, "groq");
        let info = registry.get_info(&key).unwrap();
        assert_eq!(info.name, "groq");
        assert_eq!(info.version, "1.0.0");
        assert_eq!(info.category, PluginCategory::Provider);
    }

    #[test]
    fn test_get_info_not_found() {
        let registry = PluginRegistry::new();
        let key = PluginKey::new(PluginCategory::Extension, "nope");
        assert!(registry.get_info(&key).is_none());
    }

    #[test]
    fn test_list_all() {
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(MockPlugin::new(
                "shell_exec",
                PluginCategory::Tool,
            )))
            .unwrap();
        registry
            .register(Box::new(MockPlugin::new("groq", PluginCategory::Provider)))
            .unwrap();

        let all = registry.list();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_list_by_category() {
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(MockPlugin::new(
                "shell_exec",
                PluginCategory::Tool,
            )))
            .unwrap();
        registry
            .register(Box::new(MockPlugin::new("file_ops", PluginCategory::Tool)))
            .unwrap();
        registry
            .register(Box::new(MockPlugin::new("groq", PluginCategory::Provider)))
            .unwrap();

        let tools = registry.list_by_category(&PluginCategory::Tool);
        assert_eq!(tools.len(), 2);

        let providers = registry.list_by_category(&PluginCategory::Provider);
        assert_eq!(providers.len(), 1);

        let skills = registry.list_by_category(&PluginCategory::Skill);
        assert!(skills.is_empty());
    }

    #[test]
    fn test_health_check_all() {
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(
                MockPlugin::new("healthy", PluginCategory::Tool).with_health(PluginStatus::Running),
            ))
            .unwrap();
        registry
            .register(Box::new(
                MockPlugin::new("sick", PluginCategory::Extension).with_health(PluginStatus::Error),
            ))
            .unwrap();

        let results = registry.health_check_all();
        assert_eq!(results.len(), 2);

        let statuses: Vec<_> = results.iter().map(|(_, s)| s.clone()).collect();
        assert!(statuses.contains(&PluginStatus::Running));
        assert!(statuses.contains(&PluginStatus::Error));
    }

    #[test]
    fn test_start_all() {
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(MockPlugin::new("a", PluginCategory::Tool)))
            .unwrap();
        registry
            .register(Box::new(MockPlugin::new("b", PluginCategory::Provider)))
            .unwrap();

        let results = registry.start_all();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|(_, r)| r.is_ok()));

        // Verify they report as Running after start.
        for info in registry.list() {
            assert_eq!(info.status, PluginStatus::Running);
        }
    }

    #[test]
    fn test_stop_all() {
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(MockPlugin::new("a", PluginCategory::Tool)))
            .unwrap();

        registry.start_all();
        let results = registry.stop_all();
        assert_eq!(results.len(), 1);
        assert!(results[0].1.is_ok());

        // After stop, should report as Available (not Running).
        for info in registry.list() {
            assert_eq!(info.status, PluginStatus::Available);
        }
    }

    #[test]
    fn test_start_all_with_failure() {
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(MockPlugin::new("good", PluginCategory::Tool)))
            .unwrap();
        registry.register(Box::new(FailingPlugin)).unwrap();

        let results = registry.start_all();
        assert_eq!(results.len(), 2);

        let successes = results.iter().filter(|(_, r)| r.is_ok()).count();
        let failures = results.iter().filter(|(_, r)| r.is_err()).count();
        assert_eq!(successes, 1);
        assert_eq!(failures, 1);
    }

    #[test]
    fn test_stop_all_with_failure() {
        let mut registry = PluginRegistry::new();
        registry
            .register(Box::new(MockPlugin::new("good", PluginCategory::Tool)))
            .unwrap();
        registry.register(Box::new(FailingPlugin)).unwrap();

        let results = registry.stop_all();
        let failures = results.iter().filter(|(_, r)| r.is_err()).count();
        assert_eq!(failures, 1);
    }

    #[test]
    fn test_plugin_key_display() {
        let key = PluginKey::new(PluginCategory::Tool, "shell_exec");
        let display = format!("{key}");
        assert!(display.contains("Tool"));
        assert!(display.contains("shell_exec"));
    }

    #[test]
    fn test_default_registry() {
        let registry = PluginRegistry::default();
        assert_eq!(registry.count(), 0);
    }

    #[test]
    fn test_plugin_info_serialization() {
        let info = PluginInfo {
            name: "test".to_string(),
            version: "1.0.0".to_string(),
            category: PluginCategory::Skill,
            status: PluginStatus::Available,
            description: "A test plugin".to_string(),
        };
        let json = serde_json::to_string(&info).unwrap();
        let parsed: PluginInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.name, "test");
        assert_eq!(parsed.category, PluginCategory::Skill);
    }
}
