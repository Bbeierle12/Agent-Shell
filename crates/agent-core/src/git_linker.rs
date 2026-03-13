//! Links git events to agent sessions.
//!
//! Records which git events (commits, branch switches, etc.) happened during a
//! given session, enabling reverse lookups like "which session produced this
//! commit?" Ported from ShellVault's `git::linker` and adapted to Agent-Shell's
//! `String`-based session IDs.

use crate::git_tracker::GitEvent;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A git event recorded against a specific session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitSessionLink {
    /// The git event itself.
    pub git_event: GitEvent,
    /// Session that was active when the event occurred.
    pub session_id: String,
    /// When this link was recorded.
    pub timestamp: DateTime<Utc>,
}

/// Maps git events to sessions and provides reverse lookups.
pub struct GitLinker {
    /// Events grouped by session id.
    events_by_session: HashMap<String, Vec<GitSessionLink>>,
    /// Commit hash -> session id (for fast reverse lookup).
    commit_to_session: HashMap<String, String>,
    /// All links in insertion order (for `recent_events`).
    all_links: Vec<GitSessionLink>,
}

impl GitLinker {
    pub fn new() -> Self {
        Self {
            events_by_session: HashMap::new(),
            commit_to_session: HashMap::new(),
            all_links: Vec::new(),
        }
    }

    /// Record a git event that happened during `session_id`.
    pub fn record_event(&mut self, session_id: &str, git_event: GitEvent) {
        // Index commits for reverse lookup.
        if let GitEvent::Commit { ref hash, .. } = git_event {
            self.commit_to_session
                .insert(hash.clone(), session_id.to_string());
        }

        let link = GitSessionLink {
            git_event,
            session_id: session_id.to_string(),
            timestamp: Utc::now(),
        };

        self.events_by_session
            .entry(session_id.to_string())
            .or_default()
            .push(link.clone());

        self.all_links.push(link);
    }

    /// Find which session produced a given commit hash.
    ///
    /// Supports both exact and prefix matching (e.g. short hashes).
    pub fn find_commit_session(&self, commit_hash: &str) -> Option<String> {
        // Exact match first.
        if let Some(sid) = self.commit_to_session.get(commit_hash) {
            return Some(sid.clone());
        }

        // Prefix match (short hash -> stored, or stored -> short hash).
        for (hash, sid) in &self.commit_to_session {
            if hash.starts_with(commit_hash) || commit_hash.starts_with(hash) {
                return Some(sid.clone());
            }
        }

        None
    }

    /// Get all commit events for a session.
    pub fn get_session_commits(&self, session_id: &str) -> Vec<GitEvent> {
        self.events_by_session
            .get(session_id)
            .map(|links| {
                links
                    .iter()
                    .filter_map(|l| match &l.git_event {
                        e @ GitEvent::Commit { .. } => Some(e.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get all session links for a session.
    pub fn get_session_links(&self, session_id: &str) -> Vec<GitSessionLink> {
        self.events_by_session
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Return the most recent `limit` events across all sessions, newest first.
    pub fn recent_events(&self, limit: usize) -> Vec<GitSessionLink> {
        self.all_links
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }
}

impl Default for GitLinker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_retrieve() {
        let mut linker = GitLinker::new();
        linker.record_event(
            "session-1",
            GitEvent::Commit {
                hash: "abc1234".into(),
                message: "fix bug".into(),
                author: "alice".into(),
            },
        );
        linker.record_event(
            "session-1",
            GitEvent::BranchSwitch {
                from: "main".into(),
                to: "feat".into(),
            },
        );

        let links = linker.get_session_links("session-1");
        assert_eq!(links.len(), 2);

        let commits = linker.get_session_commits("session-1");
        assert_eq!(commits.len(), 1);
        assert!(matches!(
            &commits[0],
            GitEvent::Commit { hash, .. } if hash == "abc1234"
        ));

        // Empty session returns nothing.
        assert!(linker.get_session_links("nonexistent").is_empty());
    }

    #[test]
    fn test_find_commit_session() {
        let mut linker = GitLinker::new();
        linker.record_event(
            "sess-A",
            GitEvent::Commit {
                hash: "abc1234".into(),
                message: "initial".into(),
                author: "bob".into(),
            },
        );
        linker.record_event(
            "sess-B",
            GitEvent::Commit {
                hash: "def5678".into(),
                message: "second".into(),
                author: "carol".into(),
            },
        );

        // Exact match.
        assert_eq!(
            linker.find_commit_session("abc1234"),
            Some("sess-A".into())
        );
        assert_eq!(
            linker.find_commit_session("def5678"),
            Some("sess-B".into())
        );

        // Prefix match (caller supplies longer hash that starts with stored).
        assert_eq!(
            linker.find_commit_session("abc1234deadbeef"),
            Some("sess-A".into())
        );

        // Prefix match (caller supplies shorter hash).
        assert_eq!(linker.find_commit_session("abc"), Some("sess-A".into()));

        // No match.
        assert_eq!(linker.find_commit_session("zzz"), None);
    }

    #[test]
    fn test_recent_events_ordering() {
        let mut linker = GitLinker::new();
        linker.record_event(
            "s1",
            GitEvent::Commit {
                hash: "aaa".into(),
                message: "first".into(),
                author: "a".into(),
            },
        );
        linker.record_event(
            "s2",
            GitEvent::Commit {
                hash: "bbb".into(),
                message: "second".into(),
                author: "b".into(),
            },
        );
        linker.record_event(
            "s3",
            GitEvent::Commit {
                hash: "ccc".into(),
                message: "third".into(),
                author: "c".into(),
            },
        );

        let recent = linker.recent_events(2);
        assert_eq!(recent.len(), 2);

        // Newest first.
        assert_eq!(recent[0].session_id, "s3");
        assert_eq!(recent[1].session_id, "s2");

        // Limit larger than total returns all.
        let all = linker.recent_events(100);
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_multiple_commits_same_session() {
        let mut linker = GitLinker::new();
        linker.record_event(
            "s1",
            GitEvent::Commit {
                hash: "c1".into(),
                message: "commit 1".into(),
                author: "x".into(),
            },
        );
        linker.record_event(
            "s1",
            GitEvent::Commit {
                hash: "c2".into(),
                message: "commit 2".into(),
                author: "x".into(),
            },
        );

        let commits = linker.get_session_commits("s1");
        assert_eq!(commits.len(), 2);

        assert_eq!(
            linker.find_commit_session("c1"),
            Some("s1".into())
        );
        assert_eq!(
            linker.find_commit_session("c2"),
            Some("s1".into())
        );
    }
}
