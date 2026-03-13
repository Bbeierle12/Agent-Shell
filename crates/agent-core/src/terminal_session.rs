//! Terminal session manager (in-memory).
//!
//! Tracks terminal sessions and their commands using the capture events from
//! [`agent_pty::CaptureEvent`]. Named `TerminalSession` to avoid collision with
//! the LLM conversation [`Session`](crate::session::Session).
//!
//! No database dependency -- all state is held in a `HashMap`.
//! Ported from ShellVault's `session::models`.

use crate::command_parser::{CommandParser, ParsedCommand, ToolCategory};
use agent_pty::CaptureEvent;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

/// A terminal session containing multiple commands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalSession {
    pub id: Uuid,
    pub source: SessionSource,
    pub working_directory: PathBuf,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub shell: String,
    pub terminal: Option<String>,
    pub tags: Vec<String>,
    pub notes: Option<String>,
}

impl TerminalSession {
    /// Create a new terminal session.
    pub fn new(
        id: Uuid,
        shell: String,
        working_directory: PathBuf,
        terminal: Option<String>,
        started_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            source: SessionSource::ShellHook {
                terminal: terminal.clone(),
            },
            working_directory,
            started_at,
            ended_at: None,
            shell,
            terminal,
            tags: Vec::new(),
            notes: None,
        }
    }

    /// End this session.
    pub fn end(&mut self, timestamp: DateTime<Utc>) {
        self.ended_at = Some(timestamp);
    }

    /// Calculate session duration in seconds.
    pub fn duration_secs(&self) -> Option<i64> {
        self.ended_at
            .map(|end| (end - self.started_at).num_seconds())
    }

    /// Check if session is still active.
    pub fn is_active(&self) -> bool {
        self.ended_at.is_none()
    }

    /// Add a tag to the session (no duplicates).
    pub fn add_tag(&mut self, tag: impl Into<String>) {
        let tag = tag.into();
        if !self.tags.contains(&tag) {
            self.tags.push(tag);
        }
    }
}

/// How the session was captured.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionSource {
    ShellHook { terminal: Option<String> },
    BuiltInTerminal,
    ProcessDetected { terminal: String, pid: u32 },
    Unknown,
}

/// A single command executed in a terminal session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalCommand {
    pub id: Uuid,
    pub session_id: Uuid,
    pub sequence: u32,
    pub command_text: String,
    pub working_directory: PathBuf,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub exit_code: Option<i32>,
    pub duration_ms: Option<u64>,
    pub parsed: Option<ParsedCommand>,
}

impl TerminalCommand {
    /// Create a new command.
    pub fn new(
        id: Uuid,
        session_id: Uuid,
        sequence: u32,
        command_text: String,
        working_directory: PathBuf,
        started_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id,
            session_id,
            sequence,
            command_text,
            working_directory,
            started_at,
            ended_at: None,
            exit_code: None,
            duration_ms: None,
            parsed: None,
        }
    }

    /// Complete this command with an exit code and timestamp.
    pub fn complete(&mut self, exit_code: i32, timestamp: DateTime<Utc>) {
        self.ended_at = Some(timestamp);
        self.exit_code = Some(exit_code);
        self.duration_ms = Some(
            (timestamp - self.started_at)
                .num_milliseconds()
                .unsigned_abs(),
        );
    }

    /// Check if command succeeded (exit code 0).
    pub fn succeeded(&self) -> bool {
        self.exit_code == Some(0)
    }

    /// Check if command failed (non-zero exit code).
    pub fn failed(&self) -> bool {
        self.exit_code.map(|c| c != 0).unwrap_or(false)
    }
}

/// In-memory manager for terminal sessions and their commands.
///
/// Processes [`CaptureEvent`]s to maintain current state and history.
pub struct TerminalSessionManager {
    sessions: HashMap<Uuid, TerminalSession>,
    commands: HashMap<Uuid, Vec<TerminalCommand>>,
    /// Track next command sequence number per session.
    sequence_counters: HashMap<Uuid, u32>,
    parser: CommandParser,
}

impl TerminalSessionManager {
    /// Create a new empty manager.
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            commands: HashMap::new(),
            sequence_counters: HashMap::new(),
            parser: CommandParser::new(),
        }
    }

    /// Process a capture event, updating internal state.
    pub fn process_event(&mut self, event: &CaptureEvent) {
        match event {
            CaptureEvent::SessionStart {
                session_id,
                shell,
                working_directory,
                terminal,
                timestamp,
            } => {
                let session = TerminalSession::new(
                    *session_id,
                    shell.clone(),
                    working_directory.clone(),
                    terminal.clone(),
                    *timestamp,
                );
                self.sessions.insert(*session_id, session);
                self.commands.insert(*session_id, Vec::new());
                self.sequence_counters.insert(*session_id, 0);
            }
            CaptureEvent::SessionEnd {
                session_id,
                timestamp,
            } => {
                if let Some(session) = self.sessions.get_mut(session_id) {
                    session.end(*timestamp);
                }
            }
            CaptureEvent::CommandStart {
                command_id,
                session_id,
                command_text,
                working_directory,
                timestamp,
            } => {
                let seq = self
                    .sequence_counters
                    .entry(*session_id)
                    .or_insert(0);
                *seq += 1;
                let sequence = *seq;

                let mut cmd = TerminalCommand::new(
                    *command_id,
                    *session_id,
                    sequence,
                    command_text.clone(),
                    working_directory.clone(),
                    *timestamp,
                );

                // Parse the command for tool detection.
                cmd.parsed = self.parser.parse(command_text);

                self.commands
                    .entry(*session_id)
                    .or_default()
                    .push(cmd);
            }
            CaptureEvent::CommandEnd {
                command_id,
                session_id,
                exit_code,
                timestamp,
            } => {
                if let Some(cmds) = self.commands.get_mut(session_id) {
                    if let Some(cmd) = cmds.iter_mut().find(|c| c.id == *command_id) {
                        cmd.complete(*exit_code, *timestamp);
                    }
                }
            }
            CaptureEvent::DirectoryChange {
                session_id,
                new_directory,
                ..
            } => {
                if let Some(session) = self.sessions.get_mut(session_id) {
                    session.working_directory = new_directory.clone();
                }
            }
            CaptureEvent::Output { .. } => {
                // Output events are handled by the PTY layer; no-op here.
            }
        }
    }

    /// Get a session by ID.
    pub fn get_session(&self, id: &Uuid) -> Option<&TerminalSession> {
        self.sessions.get(id)
    }

    /// Get all commands for a session.
    pub fn get_commands(&self, session_id: &Uuid) -> Option<&[TerminalCommand]> {
        self.commands.get(session_id).map(|v| v.as_slice())
    }

    /// Get all currently active sessions.
    pub fn active_sessions(&self) -> Vec<&TerminalSession> {
        self.sessions.values().filter(|s| s.is_active()).collect()
    }

    /// Get all sessions (active and ended).
    pub fn all_sessions(&self) -> Vec<&TerminalSession> {
        self.sessions.values().collect()
    }

    /// Total number of sessions tracked.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Total number of commands across all sessions.
    pub fn total_command_count(&self) -> usize {
        self.commands.values().map(|v| v.len()).sum()
    }

    /// Get the most recent N commands across all sessions, sorted by start time (newest first).
    pub fn recent_commands(&self, max: usize) -> Vec<&TerminalCommand> {
        let mut all: Vec<&TerminalCommand> = self
            .commands
            .values()
            .flat_map(|cmds| cmds.iter())
            .collect();
        all.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        all.truncate(max);
        all
    }

    /// Get commands matching a specific tool category.
    pub fn commands_by_category(&self, category: &ToolCategory) -> Vec<&TerminalCommand> {
        self.commands
            .values()
            .flat_map(|cmds| cmds.iter())
            .filter(|cmd| {
                cmd.parsed
                    .as_ref()
                    .map(|p| &p.tool_category == category)
                    .unwrap_or(false)
            })
            .collect()
    }

    /// Remove a session and its commands from memory.
    pub fn remove_session(&mut self, id: &Uuid) -> Option<TerminalSession> {
        self.commands.remove(id);
        self.sequence_counters.remove(id);
        self.sessions.remove(id)
    }

    /// Remove all ended sessions (cleanup).
    pub fn prune_ended(&mut self) -> usize {
        let ended: Vec<Uuid> = self
            .sessions
            .iter()
            .filter(|(_, s)| !s.is_active())
            .map(|(id, _)| *id)
            .collect();
        let count = ended.len();
        for id in ended {
            self.remove_session(&id);
        }
        count
    }
}

impl Default for TerminalSessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_session_start(session_id: Uuid) -> CaptureEvent {
        CaptureEvent::SessionStart {
            session_id,
            shell: "bash".to_string(),
            working_directory: PathBuf::from("/home/user"),
            terminal: Some("alacritty".to_string()),
            timestamp: Utc::now(),
        }
    }

    fn make_command_start(session_id: Uuid, command_id: Uuid, cmd: &str) -> CaptureEvent {
        CaptureEvent::CommandStart {
            command_id,
            session_id,
            command_text: cmd.to_string(),
            working_directory: PathBuf::from("/home/user"),
            timestamp: Utc::now(),
        }
    }

    fn make_command_end(session_id: Uuid, command_id: Uuid, exit_code: i32) -> CaptureEvent {
        CaptureEvent::CommandEnd {
            command_id,
            session_id,
            exit_code,
            timestamp: Utc::now(),
        }
    }

    #[test]
    fn test_session_lifecycle() {
        let mut mgr = TerminalSessionManager::new();
        let sid = Uuid::new_v4();

        mgr.process_event(&make_session_start(sid));
        assert_eq!(mgr.session_count(), 1);
        assert_eq!(mgr.active_sessions().len(), 1);

        mgr.process_event(&CaptureEvent::SessionEnd {
            session_id: sid,
            timestamp: Utc::now(),
        });
        assert_eq!(mgr.session_count(), 1);
        assert_eq!(mgr.active_sessions().len(), 0);
        assert!(!mgr.get_session(&sid).unwrap().is_active());
    }

    #[test]
    fn test_command_lifecycle() {
        let mut mgr = TerminalSessionManager::new();
        let sid = Uuid::new_v4();
        let cid = Uuid::new_v4();

        mgr.process_event(&make_session_start(sid));
        mgr.process_event(&make_command_start(sid, cid, "git status"));
        assert_eq!(mgr.total_command_count(), 1);

        let cmds = mgr.get_commands(&sid).unwrap();
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].command_text, "git status");
        assert_eq!(cmds[0].sequence, 1);
        assert!(cmds[0].exit_code.is_none());

        mgr.process_event(&make_command_end(sid, cid, 0));
        let cmds = mgr.get_commands(&sid).unwrap();
        assert!(cmds[0].succeeded());
        assert!(cmds[0].duration_ms.is_some());
    }

    #[test]
    fn test_command_parsing_integration() {
        let mut mgr = TerminalSessionManager::new();
        let sid = Uuid::new_v4();
        let cid = Uuid::new_v4();

        mgr.process_event(&make_session_start(sid));
        mgr.process_event(&make_command_start(sid, cid, "git commit -m 'test'"));

        let cmds = mgr.get_commands(&sid).unwrap();
        let parsed = cmds[0].parsed.as_ref().unwrap();
        assert_eq!(parsed.program, "git");
        assert_eq!(parsed.subcommand, Some("commit".to_string()));
        assert_eq!(parsed.tool_category, ToolCategory::VersionControl);
    }

    #[test]
    fn test_directory_change() {
        let mut mgr = TerminalSessionManager::new();
        let sid = Uuid::new_v4();

        mgr.process_event(&make_session_start(sid));
        assert_eq!(
            mgr.get_session(&sid).unwrap().working_directory,
            PathBuf::from("/home/user")
        );

        mgr.process_event(&CaptureEvent::DirectoryChange {
            session_id: sid,
            new_directory: PathBuf::from("/tmp"),
            timestamp: Utc::now(),
        });
        assert_eq!(
            mgr.get_session(&sid).unwrap().working_directory,
            PathBuf::from("/tmp")
        );
    }

    #[test]
    fn test_multiple_sessions() {
        let mut mgr = TerminalSessionManager::new();
        let s1 = Uuid::new_v4();
        let s2 = Uuid::new_v4();

        mgr.process_event(&make_session_start(s1));
        mgr.process_event(&make_session_start(s2));
        assert_eq!(mgr.session_count(), 2);
        assert_eq!(mgr.active_sessions().len(), 2);
    }

    #[test]
    fn test_sequence_numbers() {
        let mut mgr = TerminalSessionManager::new();
        let sid = Uuid::new_v4();
        let c1 = Uuid::new_v4();
        let c2 = Uuid::new_v4();
        let c3 = Uuid::new_v4();

        mgr.process_event(&make_session_start(sid));
        mgr.process_event(&make_command_start(sid, c1, "ls"));
        mgr.process_event(&make_command_start(sid, c2, "pwd"));
        mgr.process_event(&make_command_start(sid, c3, "whoami"));

        let cmds = mgr.get_commands(&sid).unwrap();
        assert_eq!(cmds[0].sequence, 1);
        assert_eq!(cmds[1].sequence, 2);
        assert_eq!(cmds[2].sequence, 3);
    }

    #[test]
    fn test_recent_commands() {
        let mut mgr = TerminalSessionManager::new();
        let sid = Uuid::new_v4();

        mgr.process_event(&make_session_start(sid));
        for i in 0..10 {
            let cid = Uuid::new_v4();
            mgr.process_event(&make_command_start(sid, cid, &format!("cmd-{}", i)));
        }

        let recent = mgr.recent_commands(3);
        assert_eq!(recent.len(), 3);
    }

    #[test]
    fn test_commands_by_category() {
        let mut mgr = TerminalSessionManager::new();
        let sid = Uuid::new_v4();

        mgr.process_event(&make_session_start(sid));
        mgr.process_event(&make_command_start(sid, Uuid::new_v4(), "git status"));
        mgr.process_event(&make_command_start(sid, Uuid::new_v4(), "ls -la"));
        mgr.process_event(&make_command_start(sid, Uuid::new_v4(), "git log"));

        let git_cmds = mgr.commands_by_category(&ToolCategory::VersionControl);
        assert_eq!(git_cmds.len(), 2);
    }

    #[test]
    fn test_remove_session() {
        let mut mgr = TerminalSessionManager::new();
        let sid = Uuid::new_v4();

        mgr.process_event(&make_session_start(sid));
        mgr.process_event(&make_command_start(sid, Uuid::new_v4(), "echo hello"));
        assert_eq!(mgr.session_count(), 1);
        assert_eq!(mgr.total_command_count(), 1);

        let removed = mgr.remove_session(&sid);
        assert!(removed.is_some());
        assert_eq!(mgr.session_count(), 0);
        assert_eq!(mgr.total_command_count(), 0);
    }

    #[test]
    fn test_prune_ended() {
        let mut mgr = TerminalSessionManager::new();
        let s1 = Uuid::new_v4();
        let s2 = Uuid::new_v4();

        mgr.process_event(&make_session_start(s1));
        mgr.process_event(&make_session_start(s2));
        mgr.process_event(&CaptureEvent::SessionEnd {
            session_id: s1,
            timestamp: Utc::now(),
        });

        let pruned = mgr.prune_ended();
        assert_eq!(pruned, 1);
        assert_eq!(mgr.session_count(), 1);
        assert!(mgr.get_session(&s2).is_some());
    }

    #[test]
    fn test_terminal_session_tags() {
        let mut session = TerminalSession::new(
            Uuid::new_v4(),
            "bash".to_string(),
            PathBuf::from("/tmp"),
            None,
            Utc::now(),
        );

        session.add_tag("dev");
        session.add_tag("dev"); // duplicate
        session.add_tag("test");
        assert_eq!(session.tags.len(), 2);
    }

    #[test]
    fn test_terminal_command_failed() {
        let mut cmd = TerminalCommand::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            1,
            "false".to_string(),
            PathBuf::from("/tmp"),
            Utc::now(),
        );

        assert!(!cmd.succeeded());
        assert!(!cmd.failed()); // no exit code yet

        cmd.complete(1, Utc::now());
        assert!(cmd.failed());
        assert!(!cmd.succeeded());
    }

    #[test]
    fn test_session_duration() {
        let start = Utc::now();
        let mut session = TerminalSession::new(
            Uuid::new_v4(),
            "zsh".to_string(),
            PathBuf::from("/home"),
            None,
            start,
        );
        assert!(session.duration_secs().is_none());

        session.end(start + chrono::Duration::seconds(42));
        assert_eq!(session.duration_secs(), Some(42));
    }

    #[test]
    fn test_output_event_is_noop() {
        let mut mgr = TerminalSessionManager::new();
        let sid = Uuid::new_v4();
        mgr.process_event(&make_session_start(sid));

        // Should not panic or change state.
        mgr.process_event(&CaptureEvent::Output {
            session_id: sid,
            data: b"hello".to_vec(),
            timestamp: Utc::now(),
        });
        assert_eq!(mgr.session_count(), 1);
    }
}
