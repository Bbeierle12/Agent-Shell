//! Shell hook capture backend.
//!
//! Receives messages from shell integrations (bash/zsh/fish hooks) via IPC and
//! converts them into [`CaptureEvent`]s. Adapted from ShellVault's
//! `capture::hook` module to use `agent_pty::CaptureEvent`.

use crate::capture::types::{
    HookMessage, HookMessageV1, LegacyHookEventType, LegacyHookMessage, HOOK_PROTOCOL_VERSION,
};
use crate::error::{AgentError, Result};
use base64::engine::general_purpose::STANDARD as BASE64_ENGINE;
use base64::Engine;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::sync::mpsc;
use uuid::Uuid;

// Re-export CaptureEvent from agent-pty so callers can use it through us.
pub use agent_pty::CaptureEvent;

/// Backend that receives messages from shell hooks via IPC.
///
/// Tracks active sessions and in-flight commands so that events carry
/// consistent session/command IDs even when the shell integration only
/// provides string-based identifiers.
pub struct HookBackend {
    active: bool,
    event_rx: Option<mpsc::Receiver<CaptureEvent>>,
    event_tx: mpsc::Sender<CaptureEvent>,
    /// Track active sessions by shell-provided session ID.
    sessions: HashMap<String, Uuid>,
    /// Track in-flight commands: key = "sid:cmd_id", value = (command_uuid, session_uuid).
    commands: HashMap<String, (Uuid, Uuid)>,
}

impl HookBackend {
    /// Create a new hook backend with an internal event channel (capacity 1000).
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(1000);
        Self {
            active: false,
            event_rx: Some(rx),
            event_tx: tx,
            sessions: HashMap::new(),
            commands: HashMap::new(),
        }
    }

    /// Get a clone of the sender for injecting hook messages externally.
    pub fn sender(&self) -> mpsc::Sender<CaptureEvent> {
        self.event_tx.clone()
    }

    /// Take the event receiver for background processing.
    ///
    /// Can only be called once; subsequent calls return `None`.
    pub fn take_receiver(&mut self) -> Option<mpsc::Receiver<CaptureEvent>> {
        self.event_rx.take()
    }

    /// Process a raw hook message from shell integration.
    pub async fn process_message(&mut self, msg: HookMessage) -> Result<()> {
        match msg {
            HookMessage::V1(msg) => self.process_v1(msg).await,
            HookMessage::Legacy(msg) => self.process_legacy(msg).await,
        }
    }

    /// Start accepting events.
    pub fn start(&mut self) {
        self.active = true;
    }

    /// Stop accepting events.
    pub fn stop(&mut self) {
        self.active = false;
    }

    /// Whether this backend is currently active.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Number of sessions currently tracked.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Number of in-flight commands currently tracked.
    pub fn command_count(&self) -> usize {
        self.commands.len()
    }

    // ---- V1 processing ----

    async fn process_v1(&mut self, msg: HookMessageV1) -> Result<()> {
        match msg {
            HookMessageV1::SessionStart {
                v,
                sid,
                shell,
                terminal,
                cwd,
                ts,
                ..
            } => {
                self.validate_version(v)?;
                let _session_id = self
                    .ensure_session(&sid, shell, cwd, terminal, timestamp_from_millis(ts))
                    .await?;
            }
            HookMessageV1::SessionEnd { v, sid, ts, .. } => {
                self.validate_version(v)?;
                if let Some(session_id) = self.sessions.remove(&sid) {
                    let event = CaptureEvent::SessionEnd {
                        session_id,
                        timestamp: timestamp_from_millis(ts),
                    };
                    self.send_event(event).await?;
                }
            }
            HookMessageV1::CommandStart {
                v,
                sid,
                cmd_id,
                cmd_b64,
                cmd,
                cwd,
                ts,
                ..
            } => {
                self.validate_version(v)?;
                let session_id = self
                    .ensure_session(&sid, None, cwd.clone(), None, timestamp_from_millis(ts))
                    .await?;

                if let Some(cmd_text) = decode_command(cmd_b64, cmd) {
                    let command_id = Uuid::new_v4();
                    let cmd_key = command_key(&sid, &cmd_id);
                    self.commands.insert(cmd_key, (command_id, session_id));

                    let event = CaptureEvent::CommandStart {
                        command_id,
                        session_id,
                        command_text: cmd_text,
                        working_directory: cwd.map(PathBuf::from).unwrap_or_default(),
                        timestamp: timestamp_from_millis(ts),
                    };
                    self.send_event(event).await?;
                }
            }
            HookMessageV1::CommandEnd {
                v,
                sid,
                cmd_id,
                exit,
                ts,
                ..
            } => {
                self.validate_version(v)?;
                let cmd_key = command_key(&sid, &cmd_id);
                if let Some((command_id, session_id)) = self.commands.remove(&cmd_key) {
                    let event = CaptureEvent::CommandEnd {
                        command_id,
                        session_id,
                        exit_code: exit.unwrap_or(-1),
                        timestamp: timestamp_from_millis(ts),
                    };
                    self.send_event(event).await?;
                }
            }
            HookMessageV1::DirectoryChange {
                v, sid, cwd, ts, ..
            } => {
                self.validate_version(v)?;
                if let Some(session_id) = self.sessions.get(&sid).copied() {
                    let event = CaptureEvent::DirectoryChange {
                        session_id,
                        new_directory: PathBuf::from(cwd),
                        timestamp: timestamp_from_millis(ts),
                    };
                    self.send_event(event).await?;
                }
            }
        }
        Ok(())
    }

    // ---- Legacy processing ----

    async fn process_legacy(&mut self, msg: LegacyHookMessage) -> Result<()> {
        let shell_id = "legacy";
        match msg.event {
            LegacyHookEventType::Session => {
                let session_id = Uuid::new_v4();
                self.sessions.insert(shell_id.to_string(), session_id);

                let event = CaptureEvent::SessionStart {
                    session_id,
                    shell: "unknown".to_string(),
                    working_directory: msg.cwd.map(PathBuf::from).unwrap_or_default(),
                    terminal: None,
                    timestamp: timestamp_from_millis(msg.time),
                };
                self.send_event(event).await?;
            }
            LegacyHookEventType::Start => {
                let session_id = self
                    .ensure_session(
                        shell_id,
                        None,
                        msg.cwd.clone(),
                        None,
                        timestamp_from_millis(msg.time),
                    )
                    .await?;

                let command_id = Uuid::new_v4();
                let cmd_key = command_key(shell_id, &msg.time.to_string());
                self.commands.insert(cmd_key, (command_id, session_id));

                if let Some(cmd_text) = msg.cmd {
                    let event = CaptureEvent::CommandStart {
                        command_id,
                        session_id,
                        command_text: cmd_text,
                        working_directory: msg.cwd.map(PathBuf::from).unwrap_or_default(),
                        timestamp: timestamp_from_millis(msg.time),
                    };
                    self.send_event(event).await?;
                }
            }
            LegacyHookEventType::End => {
                if let Some(start_time) = msg.start_time {
                    let cmd_key = command_key(shell_id, &start_time.to_string());
                    if let Some((command_id, session_id)) = self.commands.remove(&cmd_key) {
                        let event = CaptureEvent::CommandEnd {
                            command_id,
                            session_id,
                            exit_code: msg.exit.unwrap_or(-1),
                            timestamp: timestamp_from_millis(
                                msg.end_time.unwrap_or(msg.time),
                            ),
                        };
                        self.send_event(event).await?;
                    }
                }
            }
        }
        Ok(())
    }

    // ---- Helpers ----

    /// Ensure a session exists for the given shell-side session ID.
    ///
    /// If the session is already tracked, returns the existing UUID. Otherwise
    /// creates a new UUID, emits a `SessionStart` event, and tracks it.
    async fn ensure_session(
        &mut self,
        sid: &str,
        shell: Option<String>,
        cwd: Option<String>,
        terminal: Option<String>,
        timestamp: DateTime<Utc>,
    ) -> Result<Uuid> {
        if let Some(session_id) = self.sessions.get(sid).copied() {
            return Ok(session_id);
        }

        let session_id = Uuid::new_v4();
        self.sessions.insert(sid.to_string(), session_id);

        let shell = shell
            .and_then(|s| if s.trim().is_empty() { None } else { Some(s) })
            .unwrap_or_else(|| "unknown".to_string());

        let event = CaptureEvent::SessionStart {
            session_id,
            shell,
            working_directory: cwd.map(PathBuf::from).unwrap_or_default(),
            terminal,
            timestamp,
        };
        self.send_event(event).await?;

        Ok(session_id)
    }

    fn validate_version(&self, v: u8) -> Result<()> {
        if v != HOOK_PROTOCOL_VERSION {
            return Err(AgentError::Session(format!(
                "Unsupported hook protocol version: {}",
                v
            )));
        }
        Ok(())
    }

    async fn send_event(&self, event: CaptureEvent) -> Result<()> {
        self.event_tx
            .send(event)
            .await
            .map_err(|e| AgentError::Session(format!("Failed to send capture event: {}", e)))
    }
}

impl Default for HookBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert epoch-milliseconds to a `DateTime<Utc>`, falling back to now.
fn timestamp_from_millis(millis: i64) -> DateTime<Utc> {
    DateTime::from_timestamp_millis(millis).unwrap_or_else(Utc::now)
}

/// Build a composite key for tracking in-flight commands.
fn command_key(sid: &str, cmd_id: &str) -> String {
    format!("{}:{}", sid, cmd_id)
}

/// Decode a command, preferring base64-encoded over plain text.
fn decode_command(cmd_b64: Option<String>, cmd: Option<String>) -> Option<String> {
    let fallback = cmd;
    if let Some(encoded) = cmd_b64 {
        if !encoded.is_empty() {
            if let Ok(bytes) = BASE64_ENGINE.decode(encoded.as_bytes()) {
                if let Ok(text) = String::from_utf8(bytes) {
                    if !text.is_empty()
                        || fallback.as_ref().map(|s| s.is_empty()).unwrap_or(true)
                    {
                        return Some(text);
                    }
                }
            }
        }
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_new_hook_backend() {
        let backend = HookBackend::new();
        assert!(!backend.is_active());
        assert_eq!(backend.session_count(), 0);
        assert_eq!(backend.command_count(), 0);
    }

    #[tokio::test]
    async fn test_start_stop() {
        let mut backend = HookBackend::new();
        backend.start();
        assert!(backend.is_active());
        backend.stop();
        assert!(!backend.is_active());
    }

    #[tokio::test]
    async fn test_process_v1_session_start() {
        let mut backend = HookBackend::new();
        let mut rx = backend.take_receiver().unwrap();

        let msg = HookMessage::V1(HookMessageV1::SessionStart {
            v: 1,
            sid: "test-session".to_string(),
            pid: 1234,
            ppid: Some(1000),
            shell: Some("bash".to_string()),
            terminal: Some("alacritty".to_string()),
            cwd: Some("/home/user".to_string()),
            ts: 1700000000000,
        });

        backend.process_message(msg).await.unwrap();
        assert_eq!(backend.session_count(), 1);

        let event = rx.try_recv().unwrap();
        if let CaptureEvent::SessionStart { shell, terminal, .. } = &event {
            assert_eq!(shell, "bash");
            assert_eq!(terminal.as_deref(), Some("alacritty"));
        } else {
            panic!("Expected SessionStart event");
        }
    }

    #[tokio::test]
    async fn test_process_v1_command_lifecycle() {
        let mut backend = HookBackend::new();
        let mut rx = backend.take_receiver().unwrap();

        // Start session.
        let start_session = HookMessage::V1(HookMessageV1::SessionStart {
            v: 1,
            sid: "s1".to_string(),
            pid: 100,
            ppid: None,
            shell: Some("zsh".to_string()),
            terminal: None,
            cwd: Some("/tmp".to_string()),
            ts: 1700000000000,
        });
        backend.process_message(start_session).await.unwrap();
        let _session_event = rx.try_recv().unwrap();

        // Start command.
        let start_cmd = HookMessage::V1(HookMessageV1::CommandStart {
            v: 1,
            sid: "s1".to_string(),
            pid: 100,
            cmd_id: "c1".to_string(),
            cmd_b64: None,
            cmd: Some("ls -la".to_string()),
            cwd: Some("/tmp".to_string()),
            ts: 1700000001000,
        });
        backend.process_message(start_cmd).await.unwrap();
        assert_eq!(backend.command_count(), 1);

        let cmd_event = rx.try_recv().unwrap();
        if let CaptureEvent::CommandStart { command_text, .. } = &cmd_event {
            assert_eq!(command_text, "ls -la");
        } else {
            panic!("Expected CommandStart event");
        }

        // End command.
        let end_cmd = HookMessage::V1(HookMessageV1::CommandEnd {
            v: 1,
            sid: "s1".to_string(),
            pid: 100,
            cmd_id: "c1".to_string(),
            exit: Some(0),
            ts: 1700000002000,
        });
        backend.process_message(end_cmd).await.unwrap();
        assert_eq!(backend.command_count(), 0);

        let end_event = rx.try_recv().unwrap();
        if let CaptureEvent::CommandEnd { exit_code, .. } = &end_event {
            assert_eq!(*exit_code, 0);
        } else {
            panic!("Expected CommandEnd event");
        }
    }

    #[tokio::test]
    async fn test_process_v1_session_end() {
        let mut backend = HookBackend::new();
        let mut rx = backend.take_receiver().unwrap();

        // Start then end a session.
        let start = HookMessage::V1(HookMessageV1::SessionStart {
            v: 1,
            sid: "s2".to_string(),
            pid: 200,
            ppid: None,
            shell: None,
            terminal: None,
            cwd: None,
            ts: 1700000000000,
        });
        backend.process_message(start).await.unwrap();
        assert_eq!(backend.session_count(), 1);
        let _ = rx.try_recv().unwrap(); // consume SessionStart

        let end = HookMessage::V1(HookMessageV1::SessionEnd {
            v: 1,
            sid: "s2".to_string(),
            pid: 200,
            ts: 1700000010000,
        });
        backend.process_message(end).await.unwrap();
        assert_eq!(backend.session_count(), 0);

        let event = rx.try_recv().unwrap();
        assert!(matches!(event, CaptureEvent::SessionEnd { .. }));
    }

    #[tokio::test]
    async fn test_process_legacy_session() {
        let mut backend = HookBackend::new();
        let mut rx = backend.take_receiver().unwrap();

        let msg = HookMessage::Legacy(LegacyHookMessage {
            event: LegacyHookEventType::Session,
            cmd: None,
            cwd: Some("/home/user".to_string()),
            exit: None,
            time: 1700000000000,
            start_time: None,
            end_time: None,
        });

        backend.process_message(msg).await.unwrap();
        assert_eq!(backend.session_count(), 1);

        let event = rx.try_recv().unwrap();
        if let CaptureEvent::SessionStart { shell, .. } = &event {
            assert_eq!(shell, "unknown");
        } else {
            panic!("Expected SessionStart from legacy session");
        }
    }

    #[tokio::test]
    async fn test_process_legacy_command_lifecycle() {
        let mut backend = HookBackend::new();
        let mut rx = backend.take_receiver().unwrap();

        // Legacy start (auto-creates session).
        let start = HookMessage::Legacy(LegacyHookMessage {
            event: LegacyHookEventType::Start,
            cmd: Some("echo hello".to_string()),
            cwd: Some("/tmp".to_string()),
            exit: None,
            time: 1700000001000,
            start_time: None,
            end_time: None,
        });
        backend.process_message(start).await.unwrap();
        // Should have created a session and a command.
        assert_eq!(backend.session_count(), 1);
        assert_eq!(backend.command_count(), 1);
        let _ = rx.try_recv().unwrap(); // SessionStart
        let cmd_event = rx.try_recv().unwrap();
        if let CaptureEvent::CommandStart { command_text, .. } = &cmd_event {
            assert_eq!(command_text, "echo hello");
        } else {
            panic!("Expected CommandStart from legacy start");
        }

        // Legacy end.
        let end = HookMessage::Legacy(LegacyHookMessage {
            event: LegacyHookEventType::End,
            cmd: None,
            cwd: None,
            exit: Some(0),
            time: 1700000002000,
            start_time: Some(1700000001000),
            end_time: Some(1700000002000),
        });
        backend.process_message(end).await.unwrap();
        assert_eq!(backend.command_count(), 0);
        let end_event = rx.try_recv().unwrap();
        assert!(matches!(end_event, CaptureEvent::CommandEnd { .. }));
    }

    #[tokio::test]
    async fn test_bad_version_rejected() {
        let mut backend = HookBackend::new();

        let msg = HookMessage::V1(HookMessageV1::SessionStart {
            v: 99,
            sid: "bad".to_string(),
            pid: 1,
            ppid: None,
            shell: None,
            terminal: None,
            cwd: None,
            ts: 1700000000000,
        });

        let result = backend.process_message(msg).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_ensure_session_idempotent() {
        let mut backend = HookBackend::new();
        let _rx = backend.take_receiver().unwrap();

        // Two SessionStart messages with the same sid should yield the same UUID.
        let msg1 = HookMessage::V1(HookMessageV1::SessionStart {
            v: 1,
            sid: "dup".to_string(),
            pid: 10,
            ppid: None,
            shell: Some("bash".to_string()),
            terminal: None,
            cwd: None,
            ts: 1700000000000,
        });
        let msg2 = HookMessage::V1(HookMessageV1::SessionStart {
            v: 1,
            sid: "dup".to_string(),
            pid: 10,
            ppid: None,
            shell: Some("bash".to_string()),
            terminal: None,
            cwd: None,
            ts: 1700000001000,
        });
        backend.process_message(msg1).await.unwrap();
        backend.process_message(msg2).await.unwrap();

        // Still only one tracked session.
        assert_eq!(backend.session_count(), 1);
    }

    #[test]
    fn test_decode_command_b64_preferred() {
        use base64::engine::general_purpose::STANDARD;
        let encoded = STANDARD.encode("echo secret");
        let result = decode_command(Some(encoded), Some("echo plain".to_string()));
        assert_eq!(result, Some("echo secret".to_string()));
    }

    #[test]
    fn test_decode_command_fallback() {
        let result = decode_command(None, Some("echo plain".to_string()));
        assert_eq!(result, Some("echo plain".to_string()));
    }

    #[test]
    fn test_decode_command_empty_b64_uses_fallback() {
        let result = decode_command(Some(String::new()), Some("fallback".to_string()));
        assert_eq!(result, Some("fallback".to_string()));
    }

    #[test]
    fn test_decode_command_invalid_b64_uses_fallback() {
        let result = decode_command(
            Some("!!!not_valid_base64!!!".to_string()),
            Some("fallback".to_string()),
        );
        assert_eq!(result, Some("fallback".to_string()));
    }

    #[test]
    fn test_timestamp_from_millis() {
        let ts = timestamp_from_millis(1700000000000);
        assert_eq!(ts.timestamp_millis(), 1700000000000);
    }

    #[test]
    fn test_command_key_format() {
        assert_eq!(command_key("s1", "c1"), "s1:c1");
    }

    #[test]
    fn test_take_receiver_once() {
        let mut backend = HookBackend::new();
        assert!(backend.take_receiver().is_some());
        assert!(backend.take_receiver().is_none());
    }
}
