use crate::config::AppConfig;
use crate::error::AgentError;
use crate::types::Message;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// IO trait re-export for async save.
use tokio::fs as async_fs;

/// A single conversation session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub messages: Vec<Message>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Tool allowlist — if Some, only these tools can be used. If None, all tools are available.
    #[serde(default)]
    pub tool_allowlist: Option<Vec<String>>,
    /// Tool denylist — these tools are never available in this session.
    #[serde(default)]
    pub tool_denylist: Vec<String>,
    /// Arbitrary metadata attached to this session.
    #[serde(default)]
    pub metadata: HashMap<String, String>,
    /// Working directory when the session was started.
    #[serde(default)]
    pub working_directory: Option<PathBuf>,
    /// Git branch active when the session was started.
    #[serde(default)]
    pub git_branch: Option<String>,
    /// User-defined tags for categorisation.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Free-form notes attached to the session.
    #[serde(default)]
    pub notes: Option<String>,
    /// Hostname of the machine where the session was created.
    #[serde(default)]
    pub hostname: Option<String>,
    /// Profile that was active when this session was created.
    #[serde(default)]
    pub profile: Option<String>,
}

impl Session {
    pub fn new(name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            messages: Vec::new(),
            created_at: now,
            updated_at: now,
            tool_allowlist: None,
            tool_denylist: Vec::new(),
            metadata: HashMap::new(),
            working_directory: None,
            git_branch: None,
            tags: Vec::new(),
            notes: None,
            hostname: None,
            profile: None,
        }
    }

    /// Add a tag to the session (no duplicates).
    pub fn add_tag(&mut self, tag: impl Into<String>) {
        let tag = tag.into();
        if !self.tags.contains(&tag) {
            self.tags.push(tag);
        }
    }

    /// Set the notes field.
    pub fn set_notes(&mut self, notes: impl Into<String>) {
        self.notes = Some(notes.into());
    }

    /// Add a message and update the timestamp.
    pub fn push_message(&mut self, message: Message) {
        self.updated_at = Utc::now();
        self.messages.push(message);
    }

    /// Check whether a named tool is allowed in this session.
    pub fn is_tool_allowed(&self, tool_name: &str) -> bool {
        if self.tool_denylist.contains(&tool_name.to_string()) {
            return false;
        }
        match &self.tool_allowlist {
            Some(allow) => allow.contains(&tool_name.to_string()),
            None => true,
        }
    }

    /// Get the most recent N messages for the context window.
    pub fn recent_messages(&self, max: usize) -> &[Message] {
        let start = self.messages.len().saturating_sub(max);
        &self.messages[start..]
    }

    /// Persist this session to disk as JSON.
    pub fn save_to(&self, dir: &Path) -> Result<(), AgentError> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join(format!("{}.json", self.id));
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Persist this session to disk as JSON (async / non-blocking).
    ///
    /// Preferred inside async contexts; avoids blocking Tokio worker threads.
    pub async fn save_to_async(&self, dir: &Path) -> Result<(), AgentError> {
        async_fs::create_dir_all(dir).await?;
        let path = dir.join(format!("{}.json", self.id));
        let json = serde_json::to_string_pretty(self)?;
        async_fs::write(path, json).await?;
        Ok(())
    }

    /// Load a session from a JSON file.
    pub fn load_from(path: &Path) -> Result<Self, AgentError> {
        let json = std::fs::read_to_string(path)?;
        let session: Self = serde_json::from_str(&json)?;
        Ok(session)
    }

    /// Load a session from a JSON file (async / non-blocking).
    pub async fn load_from_async(path: &Path) -> Result<Self, AgentError> {
        let json = async_fs::read_to_string(path).await?;
        let session: Self = serde_json::from_str(&json)?;
        Ok(session)
    }
}

/// Manages multiple sessions with persistence.
pub struct SessionManager {
    sessions: HashMap<String, Session>,
    active_session_id: Option<String>,
    sessions_dir: PathBuf,
    max_history: usize,
    auto_save: bool,
}

impl SessionManager {
    /// Create a new session manager. Loads existing sessions from disk.
    pub fn new(config: &AppConfig) -> Result<Self, AgentError> {
        let sessions_dir = config
            .session
            .history_dir
            .clone()
            .unwrap_or_else(|| AppConfig::data_dir().join("sessions"));
        std::fs::create_dir_all(&sessions_dir)?;

        let mut manager = Self {
            sessions: HashMap::new(),
            active_session_id: None,
            sessions_dir,
            max_history: config.session.max_history,
            auto_save: config.session.auto_save,
        };
        manager.load_all()?;

        // If no sessions exist, create a default one.
        if manager.sessions.is_empty() {
            let session = Session::new("default");
            let id = session.id.clone();
            manager.sessions.insert(id.clone(), session);
            manager.active_session_id = Some(id);
            manager.save_active()?;
        } else {
            // Activate the most recently updated session.
            let most_recent = manager
                .sessions
                .values()
                .max_by_key(|s| s.updated_at)
                .map(|s| s.id.clone());
            manager.active_session_id = most_recent;
        }

        Ok(manager)
    }

    /// Load all sessions from the sessions directory.
    fn load_all(&mut self) -> Result<(), AgentError> {
        let entries = std::fs::read_dir(&self.sessions_dir)?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                match Session::load_from(&path) {
                    Ok(session) => {
                        self.sessions.insert(session.id.clone(), session);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load session from {:?}: {}", path, e);
                    }
                }
            }
        }
        Ok(())
    }

    /// Get the active session.
    pub fn active_session(&self) -> Option<&Session> {
        self.active_session_id
            .as_ref()
            .and_then(|id| self.sessions.get(id))
    }

    /// Get the active session mutably.
    pub fn active_session_mut(&mut self) -> Option<&mut Session> {
        self.active_session_id
            .as_ref()
            .and_then(|id| self.sessions.get_mut(id))
    }

    /// Get the active session ID.
    pub fn active_session_id(&self) -> Option<&str> {
        self.active_session_id.as_deref()
    }

    /// Create a new session and make it active.
    pub fn create_session(&mut self, name: impl Into<String>) -> Result<&Session, AgentError> {
        let session = Session::new(name);
        let id = session.id.clone();
        self.sessions.insert(id.clone(), session);
        self.active_session_id = Some(id.clone());
        if self.auto_save {
            self.save_session(&id)?;
        }
        Ok(self.sessions.get(&id).unwrap())
    }

    /// Switch to an existing session by ID.
    pub fn switch_session(&mut self, id: &str) -> Result<(), AgentError> {
        if self.sessions.contains_key(id) {
            self.active_session_id = Some(id.to_string());
            Ok(())
        } else {
            Err(AgentError::Session(format!("Session not found: {}", id)))
        }
    }

    /// Delete a session by ID.
    pub fn delete_session(&mut self, id: &str) -> Result<(), AgentError> {
        self.sessions.remove(id);
        let path = self.sessions_dir.join(format!("{}.json", id));
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        // If we deleted the active session, switch to another or create a new default.
        if self.active_session_id.as_deref() == Some(id) {
            self.active_session_id = self.sessions.keys().next().cloned();
            if self.active_session_id.is_none() {
                self.create_session("default")?;
            }
        }
        Ok(())
    }

    /// List all sessions as (id, name, updated_at, message_count).
    pub fn list_sessions(&self) -> Vec<(&str, &str, DateTime<Utc>, usize)> {
        let mut list: Vec<_> = self
            .sessions
            .values()
            .map(|s| {
                (
                    s.id.as_str(),
                    s.name.as_str(),
                    s.updated_at,
                    s.messages.len(),
                )
            })
            .collect();
        list.sort_by(|a, b| b.2.cmp(&a.2));
        list
    }

    /// Add a message to the active session.
    pub fn push_message(&mut self, message: Message) -> Result<(), AgentError> {
        let session = self
            .active_session_mut()
            .ok_or_else(|| AgentError::Session("No active session".into()))?;
        session.push_message(message);
        if self.auto_save {
            self.save_active()?;
        }
        Ok(())
    }

    /// Add a message to the active session (async / non-blocking save).
    ///
    /// Preferred inside async contexts (e.g. axum route handlers holding
    /// `tokio::sync::RwLock`) to avoid blocking Tokio worker threads.
    pub async fn push_message_async(&mut self, message: Message) -> Result<(), AgentError> {
        let session = self
            .active_session_mut()
            .ok_or_else(|| AgentError::Session("No active session".into()))?;
        session.push_message(message);
        if self.auto_save {
            self.save_active_async().await?;
        }
        Ok(())
    }

    /// Get the recent message history for the active session (for the context window).
    pub fn recent_messages(&self) -> Vec<&Message> {
        self.active_session()
            .map(|s| s.recent_messages(self.max_history).iter().collect())
            .unwrap_or_default()
    }

    /// Save the active session to disk.
    pub fn save_active(&self) -> Result<(), AgentError> {
        if let Some(session) = self.active_session() {
            session.save_to(&self.sessions_dir)?;
        }
        Ok(())
    }

    /// Save the active session to disk (async / non-blocking).
    ///
    /// Use this from async contexts (e.g. inside a tokio::sync::RwLock guard)
    /// to avoid blocking the Tokio runtime on disk I/O.
    pub async fn save_active_async(&self) -> Result<(), AgentError> {
        if let Some(session) = self.active_session() {
            session.save_to_async(&self.sessions_dir).await?;
        }
        Ok(())
    }

    /// Save a specific session to disk.
    fn save_session(&self, id: &str) -> Result<(), AgentError> {
        if let Some(session) = self.sessions.get(id) {
            session.save_to(&self.sessions_dir)?;
        }
        Ok(())
    }

    /// Max history setting.
    pub fn max_history(&self) -> usize {
        self.max_history
    }
}
