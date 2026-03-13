//! Hook message types for shell integration.
//!
//! Defines the versioned message protocol used by shell hooks (bash/zsh/fish
//! integrations) to report session lifecycle and command execution to
//! Agent-Shell. Ported from ShellVault's `capture::traits`.

use serde::{Deserialize, Serialize};

/// Current protocol version expected from shell hooks.
pub const HOOK_PROTOCOL_VERSION: u8 = 1;

/// Top-level hook message, supports multiple versions for compatibility.
///
/// Deserialized with `#[serde(untagged)]` so the parser tries V1 first, then
/// falls back to Legacy.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HookMessage {
    V1(HookMessageV1),
    Legacy(LegacyHookMessage),
}

/// V1 hook message variants, discriminated by `"type"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HookMessageV1 {
    SessionStart {
        v: u8,
        sid: String,
        pid: u32,
        ppid: Option<u32>,
        shell: Option<String>,
        terminal: Option<String>,
        cwd: Option<String>,
        ts: i64,
    },
    SessionEnd {
        v: u8,
        sid: String,
        pid: u32,
        ts: i64,
    },
    CommandStart {
        v: u8,
        sid: String,
        pid: u32,
        cmd_id: String,
        cmd_b64: Option<String>,
        cmd: Option<String>,
        cwd: Option<String>,
        ts: i64,
    },
    CommandEnd {
        v: u8,
        sid: String,
        pid: u32,
        cmd_id: String,
        exit: Option<i32>,
        ts: i64,
    },
    DirectoryChange {
        v: u8,
        sid: String,
        pid: u32,
        cwd: String,
        ts: i64,
    },
}

/// Legacy hook message format (pre-versioned protocol).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LegacyHookMessage {
    pub event: LegacyHookEventType,
    pub cmd: Option<String>,
    pub cwd: Option<String>,
    pub exit: Option<i32>,
    pub time: i64,
    #[serde(rename = "start")]
    pub start_time: Option<i64>,
    #[serde(rename = "end")]
    pub end_time: Option<i64>,
}

/// Event types for legacy hook messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LegacyHookEventType {
    Start,
    End,
    Session,
}

/// Strongly-typed event classification for any hook message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookEventType {
    SessionStart,
    SessionEnd,
    CommandStart,
    CommandEnd,
    DirectoryChange,
    LegacyStart,
    LegacyEnd,
    LegacySession,
}

impl HookMessage {
    /// Classify this message into a [`HookEventType`].
    pub fn event_type(&self) -> HookEventType {
        match self {
            HookMessage::V1(msg) => match msg {
                HookMessageV1::SessionStart { .. } => HookEventType::SessionStart,
                HookMessageV1::SessionEnd { .. } => HookEventType::SessionEnd,
                HookMessageV1::CommandStart { .. } => HookEventType::CommandStart,
                HookMessageV1::CommandEnd { .. } => HookEventType::CommandEnd,
                HookMessageV1::DirectoryChange { .. } => HookEventType::DirectoryChange,
            },
            HookMessage::Legacy(msg) => match msg.event {
                LegacyHookEventType::Start => HookEventType::LegacyStart,
                LegacyHookEventType::End => HookEventType::LegacyEnd,
                LegacyHookEventType::Session => HookEventType::LegacySession,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_v1_session_start_deserialize() {
        let json = r#"{
            "type": "session_start",
            "v": 1,
            "sid": "abc123",
            "pid": 1234,
            "ppid": 1000,
            "shell": "bash",
            "terminal": "alacritty",
            "cwd": "/home/user",
            "ts": 1700000000000
        }"#;
        let msg: HookMessageV1 = serde_json::from_str(json).unwrap();
        if let HookMessageV1::SessionStart { v, sid, shell, .. } = &msg {
            assert_eq!(*v, 1);
            assert_eq!(sid, "abc123");
            assert_eq!(shell.as_deref(), Some("bash"));
        } else {
            panic!("Expected SessionStart");
        }
    }

    #[test]
    fn test_v1_command_start_deserialize() {
        let json = r#"{
            "type": "command_start",
            "v": 1,
            "sid": "abc123",
            "pid": 1234,
            "cmd_id": "cmd-1",
            "cmd": "ls -la",
            "cwd": "/tmp",
            "ts": 1700000000000
        }"#;
        let msg: HookMessageV1 = serde_json::from_str(json).unwrap();
        if let HookMessageV1::CommandStart { cmd, cmd_id, .. } = &msg {
            assert_eq!(cmd.as_deref(), Some("ls -la"));
            assert_eq!(cmd_id, "cmd-1");
        } else {
            panic!("Expected CommandStart");
        }
    }

    #[test]
    fn test_legacy_message_deserialize() {
        let json = r#"{
            "event": "start",
            "cmd": "echo hello",
            "cwd": "/home/user",
            "time": 1700000000000
        }"#;
        let msg: LegacyHookMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg.event, LegacyHookEventType::Start));
        assert_eq!(msg.cmd.as_deref(), Some("echo hello"));
    }

    #[test]
    fn test_hook_message_untagged_v1() {
        let json = r#"{
            "type": "session_end",
            "v": 1,
            "sid": "abc123",
            "pid": 1234,
            "ts": 1700000001000
        }"#;
        let msg: HookMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.event_type(), HookEventType::SessionEnd);
    }

    #[test]
    fn test_hook_message_untagged_legacy() {
        let json = r#"{
            "event": "session",
            "cwd": "/tmp",
            "time": 1700000000000
        }"#;
        let msg: HookMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.event_type(), HookEventType::LegacySession);
    }

    #[test]
    fn test_hook_event_type_equality() {
        assert_eq!(HookEventType::CommandStart, HookEventType::CommandStart);
        assert_ne!(HookEventType::CommandStart, HookEventType::CommandEnd);
    }

    #[test]
    fn test_v1_directory_change_deserialize() {
        let json = r#"{
            "type": "directory_change",
            "v": 1,
            "sid": "abc123",
            "pid": 1234,
            "cwd": "/var/log",
            "ts": 1700000002000
        }"#;
        let msg: HookMessage = serde_json::from_str(json).unwrap();
        assert_eq!(msg.event_type(), HookEventType::DirectoryChange);
    }
}
