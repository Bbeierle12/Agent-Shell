//! Project detection and context linking.
//!
//! Detects project types from file markers, extracts git metadata,
//! and links sessions to their project context.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// Project identifier.
pub type ProjectId = Uuid;

/// Type of project detected from file markers.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ProjectType {
    Rust,
    Python,
    Node,
    DotNet,
    Go,
    Java,
    Mixed,
    Unknown,
}

impl ProjectType {
    /// Display name for the project type.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::Python => "Python",
            Self::Node => "Node.js",
            Self::DotNet => ".NET",
            Self::Go => "Go",
            Self::Java => "Java",
            Self::Mixed => "Mixed",
            Self::Unknown => "Unknown",
        }
    }
}

/// A detected project.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub name: String,
    pub path: PathBuf,
    pub detected_types: Vec<ProjectType>,
    pub git_remote: Option<String>,
    pub git_branch: Option<String>,
    pub created_at: DateTime<Utc>,
}

impl Project {
    /// Create a new project.
    pub fn new(name: String, path: PathBuf) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            path,
            detected_types: Vec::new(),
            git_remote: None,
            git_branch: None,
            created_at: Utc::now(),
        }
    }

    /// Primary project type (first detected, or Unknown).
    pub fn primary_type(&self) -> &ProjectType {
        self.detected_types.first().unwrap_or(&ProjectType::Unknown)
    }
}

/// Git context for a directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitContext {
    /// Current branch name.
    pub branch: Option<String>,
    /// Remote origin URL.
    pub remote: Option<String>,
    /// Whether there are uncommitted changes.
    pub is_dirty: bool,
    /// Short HEAD commit hash.
    pub head_short: Option<String>,
    /// Repository root path.
    pub repo_root: PathBuf,
}

/// Project detection markers — (filename_or_extension, ProjectType).
const PROJECT_MARKERS: &[(&str, ProjectType)] = &[
    ("Cargo.toml", ProjectType::Rust),
    ("package.json", ProjectType::Node),
    ("pyproject.toml", ProjectType::Python),
    ("requirements.txt", ProjectType::Python),
    ("setup.py", ProjectType::Python),
    ("go.mod", ProjectType::Go),
    ("pom.xml", ProjectType::Java),
    ("build.gradle", ProjectType::Java),
    (".sln", ProjectType::DotNet),
];

/// Links sessions to project context, git metadata, and environments.
pub struct ContextLinker {
    /// Cache of known projects by path.
    project_cache: HashMap<PathBuf, Project>,
}

impl ContextLinker {
    pub fn new() -> Self {
        Self {
            project_cache: HashMap::new(),
        }
    }

    /// Detect project for a given directory.
    ///
    /// Walks up the directory tree until a project root is found.
    pub fn detect_project(&mut self, directory: &Path) -> Option<&Project> {
        if self.project_cache.contains_key(directory) {
            return self.project_cache.get(directory);
        }

        let mut current = directory.to_path_buf();
        loop {
            if let Some(project) = self.detect_project_at(&current) {
                self.project_cache.insert(directory.to_path_buf(), project);
                return self.project_cache.get(directory);
            }

            if !current.pop() {
                break;
            }
        }

        None
    }

    /// Check for project markers at a specific directory.
    fn detect_project_at(&self, dir: &Path) -> Option<Project> {
        let mut detected_types = Vec::new();

        for (marker, project_type) in PROJECT_MARKERS {
            let marker_path = dir.join(marker);
            if (marker_path.exists() || Self::has_extension_match(dir, marker))
                && !detected_types.contains(project_type)
            {
                detected_types.push(project_type.clone());
            }
        }

        let git_dir = dir.join(".git");
        let has_git = git_dir.exists();

        if !detected_types.is_empty() || has_git {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());

            let mut project = Project::new(name, dir.to_path_buf());

            if detected_types.is_empty() {
                project.detected_types = vec![ProjectType::Unknown];
            } else if detected_types.len() > 1 {
                project.detected_types = vec![ProjectType::Mixed];
            } else {
                project.detected_types = detected_types;
            }

            if has_git {
                project.git_remote = Self::get_git_remote(dir);
                project.git_branch = Self::get_git_branch(dir);
            }

            return Some(project);
        }

        None
    }

    /// Check for files matching an extension pattern (e.g., ".sln").
    fn has_extension_match(dir: &Path, pattern: &str) -> bool {
        if !pattern.starts_with('.') {
            return false;
        }

        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.ends_with(pattern) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Get the git remote URL for a repository.
    pub fn get_git_remote(dir: &Path) -> Option<String> {
        let output = std::process::Command::new("git")
            .args(["remote", "get-url", "origin"])
            .current_dir(dir)
            .output()
            .ok()?;

        if output.status.success() {
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            None
        }
    }

    /// Get current git branch for a directory.
    pub fn get_git_branch(dir: &Path) -> Option<String> {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(dir)
            .output()
            .ok()?;

        if output.status.success() {
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            None
        }
    }

    /// Get full git context for a directory.
    pub fn get_git_context(dir: &Path) -> Option<GitContext> {
        // Check if this is a git repo.
        let repo_root_output = std::process::Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .current_dir(dir)
            .output()
            .ok()?;

        if !repo_root_output.status.success() {
            return None;
        }

        let repo_root = PathBuf::from(
            String::from_utf8_lossy(&repo_root_output.stdout)
                .trim()
                .to_string(),
        );

        let branch = Self::get_git_branch(dir);
        let remote = Self::get_git_remote(dir);

        // Check dirty status.
        let dirty_output = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(dir)
            .output()
            .ok();
        let is_dirty = dirty_output
            .map(|o| o.status.success() && !o.stdout.is_empty())
            .unwrap_or(false);

        // Get short HEAD.
        let head_output = std::process::Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .current_dir(dir)
            .output()
            .ok();
        let head_short = head_output
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

        Some(GitContext {
            branch,
            remote,
            is_dirty,
            head_short,
            repo_root,
        })
    }

    /// Get a cached project by path.
    pub fn get_project_by_path(&self, path: &Path) -> Option<&Project> {
        self.project_cache.get(path)
    }
}

impl Default for ContextLinker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_detect_rust_project() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();

        let mut linker = ContextLinker::new();
        let project = linker.detect_project(dir.path());

        assert!(project.is_some());
        let project = project.unwrap();
        assert!(project.detected_types.contains(&ProjectType::Rust));
    }

    #[test]
    fn test_detect_node_project() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();

        let mut linker = ContextLinker::new();
        let project = linker.detect_project(dir.path());

        assert!(project.is_some());
        let project = project.unwrap();
        assert!(project.detected_types.contains(&ProjectType::Node));
    }

    #[test]
    fn test_detect_python_project() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("pyproject.toml"), "").unwrap();

        let mut linker = ContextLinker::new();
        let project = linker.detect_project(dir.path());

        assert!(project.is_some());
        let project = project.unwrap();
        assert!(project.detected_types.contains(&ProjectType::Python));
    }

    #[test]
    fn test_detect_go_project() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example.com/foo").unwrap();

        let mut linker = ContextLinker::new();
        let project = linker.detect_project(dir.path());

        assert!(project.is_some());
        let project = project.unwrap();
        assert!(project.detected_types.contains(&ProjectType::Go));
    }

    #[test]
    fn test_no_project_empty_dir() {
        let dir = TempDir::new().unwrap();

        let mut linker = ContextLinker::new();
        let project = linker.detect_project(dir.path());

        assert!(project.is_none());
    }

    #[test]
    fn test_project_cache() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();

        let mut linker = ContextLinker::new();
        let project1 = linker.detect_project(dir.path());
        assert!(project1.is_some());
        let id1 = project1.unwrap().id;

        // Second call should return cached result.
        let project2 = linker.detect_project(dir.path());
        assert!(project2.is_some());
        assert_eq!(project2.unwrap().id, id1);
    }

    #[test]
    fn test_mixed_project() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();

        let mut linker = ContextLinker::new();
        let project = linker.detect_project(dir.path());

        assert!(project.is_some());
        let project = project.unwrap();
        assert!(project.detected_types.contains(&ProjectType::Mixed));
    }

    #[test]
    fn test_git_only_project() {
        let dir = TempDir::new().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();

        let mut linker = ContextLinker::new();
        let project = linker.detect_project(dir.path());

        assert!(project.is_some());
        let project = project.unwrap();
        assert!(project.detected_types.contains(&ProjectType::Unknown));
    }

    #[test]
    fn test_project_type_display() {
        assert_eq!(ProjectType::Rust.display_name(), "Rust");
        assert_eq!(ProjectType::Python.display_name(), "Python");
        assert_eq!(ProjectType::Node.display_name(), "Node.js");
        assert_eq!(ProjectType::Go.display_name(), "Go");
    }

    #[test]
    fn test_git_context_non_repo() {
        let dir = TempDir::new().unwrap();
        let ctx = ContextLinker::get_git_context(dir.path());
        assert!(ctx.is_none());
    }
}
