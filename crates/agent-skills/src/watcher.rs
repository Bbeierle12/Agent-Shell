//! File system watcher for skill directory changes.

use std::path::Path;
use std::sync::Arc;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tracing::{debug, error, warn};

use crate::indexer::SkillIndexer;

/// File watcher that monitors skill directory for changes.
///
/// Uses incremental updates when possible, only rebuilding
/// the affected skill's entries instead of the entire index.
pub struct FileWatcher {
    watcher: RecommendedWatcher,
}

impl FileWatcher {
    /// Create and start a new file watcher.
    pub fn new(indexer: Arc<SkillIndexer>) -> Result<Self, WatchError> {
        let indexer_clone = Arc::clone(&indexer);

        let watcher = notify::recommended_watcher(move |res: Result<notify::Event, _>| {
            match res {
                Ok(event) => {
                    // Only trigger on file modifications, creations, or deletions.
                    if !matches!(
                        event.kind,
                        notify::EventKind::Create(_)
                            | notify::EventKind::Modify(_)
                            | notify::EventKind::Remove(_)
                    ) {
                        return;
                    }

                    // Determine which skill(s) were affected.
                    let mut affected_skills = std::collections::HashSet::new();

                    for path in &event.paths {
                        if let Some(skill_name) = indexer_clone.skill_from_path(path) {
                            affected_skills.insert(skill_name);
                        }
                    }

                    if affected_skills.is_empty() {
                        debug!("File change outside skill directories, doing full reload");
                        if let Err(e) = indexer_clone.reload() {
                            error!("Failed to reload index: {}", e);
                        }
                    } else {
                        for skill_name in affected_skills {
                            debug!("Incrementally updating skill: {}", skill_name);
                            if let Err(e) = indexer_clone.update_skill(&skill_name) {
                                warn!("Failed to update skill {}: {}", skill_name, e);
                                if let Err(e) = indexer_clone.reload() {
                                    error!("Failed to reload index: {}", e);
                                }
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Watch error: {:?}", e);
                }
            }
        })
        .map_err(|e| WatchError::Setup(format!("Failed to create watcher: {}", e)))?;

        Ok(Self { watcher })
    }

    /// Start watching a directory.
    pub fn watch(&mut self, path: &Path) -> Result<(), WatchError> {
        self.watcher
            .watch(path, RecursiveMode::Recursive)
            .map_err(|e| WatchError::Watch(format!("Failed to watch {:?}: {}", path, e)))?;

        tracing::info!("Started watching skills directory: {:?}", path);
        Ok(())
    }

    /// Stop watching a path.
    pub fn unwatch(&mut self, path: &Path) -> Result<(), WatchError> {
        self.watcher
            .unwatch(path)
            .map_err(|e| WatchError::Watch(format!("Failed to unwatch {:?}: {}", path, e)))?;
        Ok(())
    }
}

/// Errors that can occur with file watching.
#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    /// Failed to initialize the file watcher.
    #[error("Failed to setup watcher: {0}")]
    Setup(String),

    /// Failed to watch a specific path.
    #[error("Failed to watch path: {0}")]
    Watch(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_watcher_creation() {
        let temp_dir = TempDir::new().unwrap();

        let skill_dir = temp_dir.path().join("test-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("_meta.json"),
            r#"{"name": "test-skill", "description": "Test"}"#,
        )
        .unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "# Test").unwrap();

        let indexer = Arc::new(SkillIndexer::new(temp_dir.path()));
        indexer.reload().unwrap();

        let mut watcher = FileWatcher::new(indexer).unwrap();
        watcher.watch(temp_dir.path()).unwrap();
    }
}
