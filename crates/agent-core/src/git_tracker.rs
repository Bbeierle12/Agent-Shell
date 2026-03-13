//! Git repository tracking using git2.
//!
//! Monitors git repositories for commits, branch switches, and file changes.
//! Ported from ShellVault's `git::tracker` with adaptations for Agent-Shell's
//! `String`-based session IDs and richer `RepoStatus`.

use git2::{Repository, StatusOptions};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Events emitted when a tracked repository changes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum GitEvent {
    /// A new commit was detected.
    Commit {
        hash: String,
        message: String,
        author: String,
    },
    /// The active branch changed.
    BranchSwitch { from: String, to: String },
    /// A file's working-tree or index status changed.
    FileChanged { path: String, status: String },
}

/// Internal state for a single tracked repository.
struct TrackedRepo {
    /// Resolved working-directory path (canonical key).
    path: PathBuf,
    /// Last-known HEAD OID as hex string.
    last_known_head: Option<String>,
    /// Last-known branch name (short).
    last_known_branch: Option<String>,
}

/// Summary of a repository's current status.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoStatus {
    /// Current branch (short name).
    pub branch: Option<String>,
    /// Remote URL (origin).
    pub remote: Option<String>,
    /// Commits ahead of upstream.
    pub ahead: usize,
    /// Commits behind upstream.
    pub behind: usize,
    /// Files staged in the index.
    pub staged: usize,
    /// Files modified in the working tree.
    pub modified: usize,
    /// Untracked files.
    pub untracked: usize,
    /// Whether any paths are in conflict.
    pub has_conflicts: bool,
}

impl RepoStatus {
    /// True when the working directory has no changes at all.
    pub fn is_clean(&self) -> bool {
        self.staged == 0 && self.modified == 0 && self.untracked == 0 && !self.has_conflicts
    }
}

/// Tracks one or more git repositories and detects changes between polls.
pub struct GitTracker {
    repos: HashMap<PathBuf, TrackedRepo>,
}

impl GitTracker {
    pub fn new() -> Self {
        Self {
            repos: HashMap::new(),
        }
    }

    /// Start tracking a repository.
    ///
    /// `path` can be any directory inside the repository; the tracker will
    /// resolve the workdir root via `Repository::discover`.
    pub fn track(&mut self, path: &Path) -> Result<(), git2::Error> {
        let repo = Repository::discover(path)?;
        let repo_path = repo
            .workdir()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| path.to_path_buf());

        let head_oid = repo
            .head()
            .ok()
            .and_then(|h| h.target())
            .map(|oid| oid.to_string());

        let branch = repo
            .head()
            .ok()
            .and_then(|h| h.shorthand().map(String::from));

        self.repos.insert(
            repo_path.clone(),
            TrackedRepo {
                path: repo_path,
                last_known_head: head_oid,
                last_known_branch: branch,
            },
        );

        Ok(())
    }

    /// Stop tracking a repository.
    pub fn untrack(&mut self, path: &Path) {
        // Try to resolve the canonical workdir, falling back to exact match.
        if let Ok(repo) = Repository::discover(path) {
            if let Some(workdir) = repo.workdir() {
                self.repos.remove(workdir);
                return;
            }
        }
        self.repos.remove(path);
    }

    /// Poll all tracked repositories and return any detected events.
    ///
    /// Each event is paired with the repo path (as a `String`) so callers know
    /// which repository it came from.
    pub fn check_changes(&mut self) -> Vec<(String, GitEvent)> {
        let mut events = Vec::new();

        for tracked in self.repos.values_mut() {
            let repo = match Repository::open(&tracked.path) {
                Ok(r) => r,
                Err(_) => continue,
            };

            let head_ref = match repo.head() {
                Ok(h) => h,
                Err(_) => continue,
            };

            let current_head = head_ref.target().map(|oid| oid.to_string());
            let current_branch = head_ref.shorthand().map(String::from);
            let repo_key = tracked.path.to_string_lossy().to_string();

            // Detect branch switch.
            if current_branch != tracked.last_known_branch {
                if let (Some(from), Some(to)) = (&tracked.last_known_branch, &current_branch) {
                    events.push((
                        repo_key.clone(),
                        GitEvent::BranchSwitch {
                            from: from.clone(),
                            to: to.clone(),
                        },
                    ));
                }
                tracked.last_known_branch = current_branch;
            }

            // Detect new commit.
            if current_head != tracked.last_known_head {
                if let Some(oid_str) = &current_head {
                    if let Ok(oid) = git2::Oid::from_str(oid_str) {
                        if let Ok(commit) = repo.find_commit(oid) {
                            let message = commit
                                .message()
                                .unwrap_or("")
                                .lines()
                                .next()
                                .unwrap_or("")
                                .to_string();
                            let author = commit.author().name().unwrap_or("unknown").to_string();

                            events.push((
                                repo_key.clone(),
                                GitEvent::Commit {
                                    hash: oid_str.chars().take(7).collect(),
                                    message,
                                    author,
                                },
                            ));
                        }
                    }
                }
                tracked.last_known_head = current_head;
            }
        }

        events
    }

    /// Get detailed status for one tracked (or discoverable) repository.
    pub fn status(&self, path: &Path) -> Result<RepoStatus, git2::Error> {
        let repo = Repository::discover(path)?;

        let branch = repo
            .head()
            .ok()
            .and_then(|h| h.shorthand().map(String::from));

        let remote = repo
            .find_remote("origin")
            .ok()
            .and_then(|r| r.url().map(String::from));

        // Ahead / behind (only when we have both local and upstream).
        let (ahead, behind) = Self::ahead_behind(&repo).unwrap_or((0, 0));

        let mut opts = StatusOptions::new();
        opts.include_untracked(true);
        let statuses = repo.statuses(Some(&mut opts))?;

        let mut staged = 0usize;
        let mut modified = 0usize;
        let mut untracked = 0usize;
        let mut has_conflicts = false;

        for entry in statuses.iter() {
            let s = entry.status();
            if s.is_index_new() || s.is_index_modified() || s.is_index_deleted() {
                staged += 1;
            }
            if s.is_wt_modified() || s.is_wt_deleted() {
                modified += 1;
            }
            if s.is_wt_new() {
                untracked += 1;
            }
            if s.is_conflicted() {
                has_conflicts = true;
            }
        }

        Ok(RepoStatus {
            branch,
            remote,
            ahead,
            behind,
            staged,
            modified,
            untracked,
            has_conflicts,
        })
    }

    /// Get the current branch for a path.
    pub fn current_branch(&self, path: &Path) -> Result<String, git2::Error> {
        let repo = Repository::discover(path)?;
        let head = repo.head()?;
        Ok(head
            .shorthand()
            .map(String::from)
            .unwrap_or_else(|| "HEAD".to_string()))
    }

    /// List paths of all currently tracked repositories.
    pub fn list_tracked(&self) -> Vec<String> {
        self.repos
            .keys()
            .map(|p| p.to_string_lossy().to_string())
            .collect()
    }

    // ---- internal helpers ----

    fn ahead_behind(repo: &Repository) -> Option<(usize, usize)> {
        let head = repo.head().ok()?;
        let branch_name = head.shorthand()?;
        let local_oid = head.target()?;
        let upstream_ref_name = format!("refs/remotes/origin/{}", branch_name);
        let upstream = repo.find_reference(&upstream_ref_name).ok()?;
        let upstream_oid = upstream.target()?;
        repo.graph_ahead_behind(local_oid, upstream_oid).ok()
    }
}

impl Default for GitTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    /// Helper: create a real git repository in a temp directory.
    fn init_repo(dir: &Path) {
        Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .expect("git init failed");
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir)
            .output()
            .expect("git config email failed");
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .output()
            .expect("git config name failed");
    }

    fn commit(dir: &Path, msg: &str) {
        Command::new("git")
            .args(["add", "-A"])
            .current_dir(dir)
            .output()
            .expect("git add failed");
        Command::new("git")
            .args(["commit", "-m", msg, "--allow-empty-message"])
            .current_dir(dir)
            .output()
            .expect("git commit failed");
    }

    #[test]
    fn test_track_and_status() {
        let tmp = TempDir::new().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("hello.txt"), "world").unwrap();
        commit(tmp.path(), "initial");

        let mut tracker = GitTracker::new();
        tracker.track(tmp.path()).unwrap();

        let status = tracker.status(tmp.path()).unwrap();
        assert!(status.branch.is_some());
        assert!(status.is_clean());
    }

    #[test]
    fn test_detect_commit() {
        let tmp = TempDir::new().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("a.txt"), "first").unwrap();
        commit(tmp.path(), "first commit");

        let mut tracker = GitTracker::new();
        tracker.track(tmp.path()).unwrap();

        // Make a second commit.
        fs::write(tmp.path().join("b.txt"), "second").unwrap();
        commit(tmp.path(), "second commit");

        let events = tracker.check_changes();
        assert!(!events.is_empty());

        let has_commit = events.iter().any(|(_, e)| {
            matches!(e, GitEvent::Commit { message, .. } if message == "second commit")
        });
        assert!(has_commit, "expected Commit event, got: {:?}", events);
    }

    #[test]
    fn test_detect_branch_switch() {
        let tmp = TempDir::new().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("init.txt"), "x").unwrap();
        commit(tmp.path(), "init");

        let mut tracker = GitTracker::new();
        tracker.track(tmp.path()).unwrap();

        // Create and switch to a new branch.
        Command::new("git")
            .args(["checkout", "-b", "feature-x"])
            .current_dir(tmp.path())
            .output()
            .expect("checkout failed");

        let events = tracker.check_changes();
        let has_switch = events.iter().any(|(_, e)| {
            matches!(e, GitEvent::BranchSwitch { to, .. } if to == "feature-x")
        });
        assert!(
            has_switch,
            "expected BranchSwitch event, got: {:?}",
            events
        );
    }

    #[test]
    fn test_status_counts() {
        let tmp = TempDir::new().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("tracked.txt"), "tracked").unwrap();
        commit(tmp.path(), "init");

        // Create conditions: staged, modified, untracked.
        fs::write(tmp.path().join("new_staged.txt"), "staged").unwrap();
        Command::new("git")
            .args(["add", "new_staged.txt"])
            .current_dir(tmp.path())
            .output()
            .unwrap();
        fs::write(tmp.path().join("tracked.txt"), "modified content").unwrap();
        fs::write(tmp.path().join("untracked.txt"), "untracked").unwrap();

        let tracker = GitTracker::new();
        let status = tracker.status(tmp.path()).unwrap();

        assert_eq!(status.staged, 1, "staged count");
        assert_eq!(status.modified, 1, "modified count");
        assert_eq!(status.untracked, 1, "untracked count");
        assert!(!status.is_clean());
    }

    #[test]
    fn test_untrack_removes_repo() {
        let tmp = TempDir::new().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("x.txt"), "x").unwrap();
        commit(tmp.path(), "init");

        let mut tracker = GitTracker::new();
        tracker.track(tmp.path()).unwrap();
        assert_eq!(tracker.list_tracked().len(), 1);

        tracker.untrack(tmp.path());
        assert!(tracker.list_tracked().is_empty());
    }

    #[test]
    fn test_current_branch() {
        let tmp = TempDir::new().unwrap();
        init_repo(tmp.path());
        fs::write(tmp.path().join("f.txt"), "f").unwrap();
        commit(tmp.path(), "init");

        let tracker = GitTracker::new();
        let branch = tracker.current_branch(tmp.path()).unwrap();
        // Default branch is either "main" or "master" depending on config.
        assert!(
            branch == "main" || branch == "master",
            "unexpected branch: {}",
            branch
        );
    }

    #[test]
    fn test_list_tracked_multiple() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();
        init_repo(tmp1.path());
        init_repo(tmp2.path());
        fs::write(tmp1.path().join("a.txt"), "a").unwrap();
        commit(tmp1.path(), "init");
        fs::write(tmp2.path().join("b.txt"), "b").unwrap();
        commit(tmp2.path(), "init");

        let mut tracker = GitTracker::new();
        tracker.track(tmp1.path()).unwrap();
        tracker.track(tmp2.path()).unwrap();

        assert_eq!(tracker.list_tracked().len(), 2);
    }
}
