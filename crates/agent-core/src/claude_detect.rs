//! Claude Code process detection.
//!
//! Detects running Claude Code processes and tracks file changes
//! attributed to Claude sessions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Claude Code process names to detect.
const CLAUDE_PROCESS_NAMES: &[&str] = &["claude", "claude.exe", "claude-code"];

/// Information about a detected Claude Code session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeSession {
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    pub working_directory: PathBuf,
    pub conversation_id: Option<String>,
}

/// File modification attributed to Claude Code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeFileChange {
    pub path: PathBuf,
    pub change_type: FileChangeType,
    pub timestamp: DateTime<Utc>,
    pub session_pid: u32,
}

/// Type of file change.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileChangeType {
    Created,
    Modified,
    Deleted,
}

/// Detector for Claude Code processes.
///
/// Scans the process table for Claude Code instances and tracks
/// which sessions are active.
pub struct ClaudeDetector {
    active_sessions: HashMap<u32, ClaudeSession>,
    file_changes: Vec<ClaudeFileChange>,
}

impl ClaudeDetector {
    pub fn new() -> Self {
        Self {
            active_sessions: HashMap::new(),
            file_changes: Vec::new(),
        }
    }

    /// Check if a process name matches Claude Code.
    pub fn is_claude_process(name: &str) -> bool {
        let name_lower = name.to_lowercase();
        CLAUDE_PROCESS_NAMES.iter().any(|n| name_lower.contains(n))
    }

    /// Scan for Claude Code processes using the system process list.
    ///
    /// Returns newly discovered sessions since the last scan.
    pub fn scan(&mut self) -> Vec<ClaudeSession> {
        let mut new_sessions = Vec::new();

        // Read /proc on Linux to find Claude processes.
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let pid_str = entry.file_name();
                let pid_str = pid_str.to_string_lossy();

                // Skip non-PID directories.
                let pid: u32 = match pid_str.parse() {
                    Ok(p) => p,
                    Err(_) => continue,
                };

                if self.active_sessions.contains_key(&pid) {
                    continue;
                }

                // Read the command line.
                let cmdline_path = entry.path().join("cmdline");
                if let Ok(cmdline) = std::fs::read_to_string(&cmdline_path) {
                    let name = cmdline
                        .split('\0')
                        .next()
                        .unwrap_or("")
                        .rsplit('/')
                        .next()
                        .unwrap_or("");

                    if Self::is_claude_process(name) {
                        // Try to get the working directory.
                        let cwd = std::fs::read_link(entry.path().join("cwd"))
                            .unwrap_or_default();

                        let session = ClaudeSession {
                            pid,
                            started_at: Utc::now(),
                            working_directory: cwd,
                            conversation_id: None,
                        };
                        self.active_sessions.insert(pid, session.clone());
                        new_sessions.push(session);
                    }
                }
            }
        }

        // Clean up ended sessions — remove PIDs that no longer exist.
        self.active_sessions
            .retain(|pid, _| PathBuf::from(format!("/proc/{}", pid)).exists());

        new_sessions
    }

    /// Get all active Claude Code sessions.
    pub fn active_sessions(&self) -> &HashMap<u32, ClaudeSession> {
        &self.active_sessions
    }

    /// Get active session count.
    pub fn active_count(&self) -> usize {
        self.active_sessions.len()
    }

    /// Record a file change attributed to a Claude session.
    pub fn record_file_change(
        &mut self,
        session_pid: u32,
        path: PathBuf,
        change_type: FileChangeType,
    ) {
        if self.active_sessions.contains_key(&session_pid) {
            self.file_changes.push(ClaudeFileChange {
                path,
                change_type,
                timestamp: Utc::now(),
                session_pid,
            });
        }
    }

    /// Get file changes for a specific session.
    pub fn get_file_changes(&self, session_pid: u32) -> Vec<&ClaudeFileChange> {
        self.file_changes
            .iter()
            .filter(|c| c.session_pid == session_pid)
            .collect()
    }

    /// Get all file changes.
    pub fn all_file_changes(&self) -> &[ClaudeFileChange] {
        &self.file_changes
    }
}

impl Default for ClaudeDetector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_claude_process() {
        assert!(ClaudeDetector::is_claude_process("claude"));
        assert!(ClaudeDetector::is_claude_process("Claude"));
        assert!(ClaudeDetector::is_claude_process("claude.exe"));
        assert!(ClaudeDetector::is_claude_process("claude-code"));
        assert!(!ClaudeDetector::is_claude_process("bash"));
        assert!(!ClaudeDetector::is_claude_process("vim"));
        assert!(!ClaudeDetector::is_claude_process("node"));
    }

    #[test]
    fn test_detector_new() {
        let detector = ClaudeDetector::new();
        assert_eq!(detector.active_count(), 0);
        assert!(detector.all_file_changes().is_empty());
    }

    #[test]
    fn test_record_file_change() {
        let mut detector = ClaudeDetector::new();
        // Insert a fake session for testing.
        detector.active_sessions.insert(
            12345,
            ClaudeSession {
                pid: 12345,
                started_at: Utc::now(),
                working_directory: PathBuf::from("/tmp"),
                conversation_id: None,
            },
        );

        detector.record_file_change(
            12345,
            PathBuf::from("/tmp/test.rs"),
            FileChangeType::Modified,
        );

        assert_eq!(detector.all_file_changes().len(), 1);
        assert_eq!(detector.get_file_changes(12345).len(), 1);
        assert_eq!(detector.get_file_changes(99999).len(), 0);
    }

    #[test]
    fn test_record_file_change_unknown_session() {
        let mut detector = ClaudeDetector::new();
        // Should silently ignore changes for unknown sessions.
        detector.record_file_change(
            99999,
            PathBuf::from("/tmp/test.rs"),
            FileChangeType::Created,
        );
        assert!(detector.all_file_changes().is_empty());
    }

    #[test]
    fn test_claude_session_serialization() {
        let session = ClaudeSession {
            pid: 1234,
            started_at: Utc::now(),
            working_directory: PathBuf::from("/home/user/project"),
            conversation_id: Some("conv-123".to_string()),
        };
        let json = serde_json::to_string(&session).unwrap();
        let deserialized: ClaudeSession = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.pid, 1234);
        assert_eq!(
            deserialized.conversation_id,
            Some("conv-123".to_string())
        );
    }
}
