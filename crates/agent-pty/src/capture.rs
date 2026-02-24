//! Capture events for PTY session activity.
//!
//! Defines a unified event type for session lifecycle, command execution,
//! directory changes, and output capture. Adapted from ShellVault.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Event emitted during PTY session activity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CaptureEvent {
    /// A new PTY session has started.
    SessionStart {
        session_id: Uuid,
        shell: String,
        working_directory: PathBuf,
        terminal: Option<String>,
        timestamp: DateTime<Utc>,
    },

    /// A PTY session has ended.
    SessionEnd {
        session_id: Uuid,
        timestamp: DateTime<Utc>,
    },

    /// A command has started execution.
    CommandStart {
        command_id: Uuid,
        session_id: Uuid,
        command_text: String,
        working_directory: PathBuf,
        timestamp: DateTime<Utc>,
    },

    /// A command has finished execution.
    CommandEnd {
        command_id: Uuid,
        session_id: Uuid,
        exit_code: i32,
        timestamp: DateTime<Utc>,
    },

    /// Working directory changed.
    DirectoryChange {
        session_id: Uuid,
        new_directory: PathBuf,
        timestamp: DateTime<Utc>,
    },

    /// Raw output captured from the PTY.
    Output {
        session_id: Uuid,
        data: Vec<u8>,
        timestamp: DateTime<Utc>,
    },
}

impl CaptureEvent {
    /// Create a SessionStart event.
    pub fn session_start(
        session_id: Uuid,
        shell: impl Into<String>,
        working_directory: impl Into<PathBuf>,
    ) -> Self {
        Self::SessionStart {
            session_id,
            shell: shell.into(),
            working_directory: working_directory.into(),
            terminal: None,
            timestamp: Utc::now(),
        }
    }

    /// Create a SessionEnd event.
    pub fn session_end(session_id: Uuid) -> Self {
        Self::SessionEnd {
            session_id,
            timestamp: Utc::now(),
        }
    }

    /// Get the session ID from any event variant.
    pub fn session_id(&self) -> Uuid {
        match self {
            Self::SessionStart { session_id, .. }
            | Self::SessionEnd { session_id, .. }
            | Self::CommandStart { session_id, .. }
            | Self::CommandEnd { session_id, .. }
            | Self::DirectoryChange { session_id, .. }
            | Self::Output { session_id, .. } => *session_id,
        }
    }

    /// Get the timestamp from any event variant.
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            Self::SessionStart { timestamp, .. }
            | Self::SessionEnd { timestamp, .. }
            | Self::CommandStart { timestamp, .. }
            | Self::CommandEnd { timestamp, .. }
            | Self::DirectoryChange { timestamp, .. }
            | Self::Output { timestamp, .. } => *timestamp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_start_event() {
        let id = Uuid::new_v4();
        let event = CaptureEvent::session_start(id, "bash", "/home/user");
        assert_eq!(event.session_id(), id);

        if let CaptureEvent::SessionStart { shell, .. } = &event {
            assert_eq!(shell, "bash");
        } else {
            panic!("Expected SessionStart");
        }
    }

    #[test]
    fn test_session_end_event() {
        let id = Uuid::new_v4();
        let event = CaptureEvent::session_end(id);
        assert_eq!(event.session_id(), id);

        assert!(matches!(event, CaptureEvent::SessionEnd { .. }));
    }

    #[test]
    fn test_event_serialization_roundtrip() {
        let id = Uuid::new_v4();
        let event = CaptureEvent::session_start(id, "zsh", "/tmp");

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("session_start"));
        assert!(json.contains("zsh"));

        let parsed: CaptureEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.session_id(), id);
    }

    #[test]
    fn test_command_event_serialization() {
        let sid = Uuid::new_v4();
        let cid = Uuid::new_v4();
        let event = CaptureEvent::CommandStart {
            command_id: cid,
            session_id: sid,
            command_text: "ls -la".to_string(),
            working_directory: PathBuf::from("/home"),
            timestamp: Utc::now(),
        };

        let json = serde_json::to_string(&event).unwrap();
        let parsed: CaptureEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.session_id(), sid);

        if let CaptureEvent::CommandStart { command_text, .. } = &parsed {
            assert_eq!(command_text, "ls -la");
        } else {
            panic!("Expected CommandStart");
        }
    }

    #[test]
    fn test_output_event() {
        let sid = Uuid::new_v4();
        let event = CaptureEvent::Output {
            session_id: sid,
            data: b"hello world\n".to_vec(),
            timestamp: Utc::now(),
        };

        assert_eq!(event.session_id(), sid);
        let json = serde_json::to_string(&event).unwrap();
        let parsed: CaptureEvent = serde_json::from_str(&json).unwrap();
        if let CaptureEvent::Output { data, .. } = parsed {
            assert_eq!(data, b"hello world\n");
        } else {
            panic!("Expected Output");
        }
    }

    #[test]
    fn test_directory_change_event() {
        let sid = Uuid::new_v4();
        let event = CaptureEvent::DirectoryChange {
            session_id: sid,
            new_directory: PathBuf::from("/var/log"),
            timestamp: Utc::now(),
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("directory_change"));
        assert!(json.contains("/var/log"));
    }
}
