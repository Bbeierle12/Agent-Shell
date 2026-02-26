use agent_core::error::AgentError;
use agent_core::tool_registry::Tool;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Validate that a path is within the allowed workspace root.
/// Returns the canonicalized absolute path if valid.
pub(crate) fn validate_path(
    raw: &str,
    workspace_root: &Option<PathBuf>,
) -> Result<PathBuf, AgentError> {
    let root = match workspace_root {
        Some(r) => r,
        None => return Ok(PathBuf::from(raw)), // No restriction
    };

    // Make path absolute.
    let abs = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(raw)
    };

    // Canonicalize what exists; for non-existent paths walk up to find an
    // existing ancestor and append the remaining components.
    let canonical = if abs.exists() {
        abs.canonicalize().map_err(|e| AgentError::ToolExecution {
            tool_name: "file_ops".into(),
            message: format!("Failed to canonicalize path: {}", e),
        })?
    } else {
        // Walk up until we find an existing ancestor.
        let mut existing = abs.as_path();
        let mut tail = Vec::new();
        loop {
            if existing.exists() {
                break;
            }
            if let Some(file_name) = existing.file_name() {
                tail.push(file_name.to_owned());
                existing = existing.parent().unwrap_or(Path::new("/"));
            } else {
                break;
            }
        }
        let mut canon = existing
            .canonicalize()
            .map_err(|e| AgentError::ToolExecution {
                tool_name: "file_ops".into(),
                message: format!("Failed to canonicalize path: {}", e),
            })?;
        for component in tail.into_iter().rev() {
            canon.push(component);
        }
        canon
    };

    let canon_root = root.canonicalize().map_err(|e| AgentError::ToolExecution {
        tool_name: "file_ops".into(),
        message: format!("Failed to canonicalize workspace_root: {}", e),
    })?;

    if !canonical.starts_with(&canon_root) {
        return Err(AgentError::ToolExecution {
            tool_name: "file_ops".into(),
            message: format!(
                "Path '{}' is outside the workspace root '{}'",
                canonical.display(),
                canon_root.display()
            ),
        });
    }

    Ok(canonical)
}

// ── file_read ──────────────────────────────────────────────────────────

pub struct FileReadTool {
    pub workspace_root: Option<PathBuf>,
}

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn description(&self) -> &str {
        "Read the contents of a file. Returns the file's text content. \
         Use this to inspect source code, configuration files, logs, etc."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the file to read"
                },
                "start_line": {
                    "type": "integer",
                    "description": "Optional 1-based start line (inclusive)"
                },
                "end_line": {
                    "type": "integer",
                    "description": "Optional 1-based end line (inclusive)"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: Value) -> Result<String, AgentError> {
        #[derive(Deserialize)]
        struct Args {
            path: String,
            start_line: Option<usize>,
            end_line: Option<usize>,
        }
        let args: Args = serde_json::from_value(args).map_err(|e| AgentError::ToolExecution {
            tool_name: "file_read".into(),
            message: format!("Invalid arguments: {}", e),
        })?;

        let validated_path = validate_path(&args.path, &self.workspace_root)?;

        let content = tokio::fs::read_to_string(&validated_path)
            .await
            .map_err(|e| AgentError::ToolExecution {
                tool_name: "file_read".into(),
                message: format!("Failed to read {}: {}", args.path, e),
            })?;

        // Apply line range if specified.
        match (args.start_line, args.end_line) {
            (Some(start), Some(end)) => {
                let lines: Vec<&str> = content.lines().collect();
                let start = start.saturating_sub(1).min(lines.len());
                let end = end.min(lines.len());
                Ok(lines[start..end].join("\n"))
            }
            (Some(start), None) => {
                let lines: Vec<&str> = content.lines().collect();
                let start = start.saturating_sub(1).min(lines.len());
                Ok(lines[start..].join("\n"))
            }
            _ => Ok(content),
        }
    }
}

// ── file_write ─────────────────────────────────────────────────────────

pub struct FileWriteTool {
    pub workspace_root: Option<PathBuf>,
}

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. \
         Creates parent directories as needed."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                },
                "append": {
                    "type": "boolean",
                    "description": "If true, append to the file instead of overwriting. Default: false"
                }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(&self, args: Value) -> Result<String, AgentError> {
        #[derive(Deserialize)]
        struct Args {
            path: String,
            content: String,
            #[serde(default)]
            append: bool,
        }
        let args: Args = serde_json::from_value(args).map_err(|e| AgentError::ToolExecution {
            tool_name: "file_write".into(),
            message: format!("Invalid arguments: {}", e),
        })?;

        let validated_path = validate_path(&args.path, &self.workspace_root)?;

        if let Some(parent) = validated_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AgentError::ToolExecution {
                    tool_name: "file_write".into(),
                    message: format!("Failed to create directories: {}", e),
                })?;
        }

        if args.append {
            use tokio::io::AsyncWriteExt;
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&validated_path)
                .await
                .map_err(|e| AgentError::ToolExecution {
                    tool_name: "file_write".into(),
                    message: format!("Failed to open {}: {}", args.path, e),
                })?;
            file.write_all(args.content.as_bytes()).await.map_err(|e| {
                AgentError::ToolExecution {
                    tool_name: "file_write".into(),
                    message: format!("Failed to write: {}", e),
                }
            })?;
        } else {
            tokio::fs::write(&validated_path, &args.content)
                .await
                .map_err(|e| AgentError::ToolExecution {
                    tool_name: "file_write".into(),
                    message: format!("Failed to write {}: {}", args.path, e),
                })?;
        }

        let bytes = args.content.len();
        Ok(format!("Wrote {} bytes to {}", bytes, args.path))
    }
}

// ── file_list ──────────────────────────────────────────────────────────

pub struct FileListTool {
    pub workspace_root: Option<PathBuf>,
}

#[async_trait]
impl Tool for FileListTool {
    fn name(&self) -> &str {
        "file_list"
    }

    fn description(&self) -> &str {
        "List files and directories at a given path. Returns names with a trailing / for directories."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The directory path to list. Defaults to current directory."
                },
                "recursive": {
                    "type": "boolean",
                    "description": "If true, list recursively. Default: false"
                }
            },
            "required": []
        })
    }

    async fn execute(&self, args: Value) -> Result<String, AgentError> {
        #[derive(Deserialize)]
        struct Args {
            #[serde(default = "default_path")]
            path: String,
            #[serde(default)]
            recursive: bool,
        }
        fn default_path() -> String {
            ".".into()
        }

        let args: Args = serde_json::from_value(args).map_err(|e| AgentError::ToolExecution {
            tool_name: "file_list".into(),
            message: format!("Invalid arguments: {}", e),
        })?;

        let validated_path = validate_path(&args.path, &self.workspace_root)?;
        let path_str = validated_path.to_string_lossy().to_string();

        if args.recursive {
            list_recursive(&path_str, &self.workspace_root).await
        } else {
            list_flat(&path_str).await
        }
    }
}

async fn list_flat(path: &str) -> Result<String, AgentError> {
    let mut entries = tokio::fs::read_dir(path)
        .await
        .map_err(|e| AgentError::ToolExecution {
            tool_name: "file_list".into(),
            message: format!("Failed to read directory {}: {}", path, e),
        })?;

    let mut names = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| AgentError::ToolExecution {
            tool_name: "file_list".into(),
            message: format!("Failed to read entry: {}", e),
        })?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        let meta = entry.metadata().await.ok();
        if meta.map(|m| m.is_dir()).unwrap_or(false) {
            names.push(format!("{}/", name));
        } else {
            names.push(name);
        }
    }
    names.sort();
    Ok(names.join("\n"))
}

/// Maximum recursion depth to prevent runaway traversals.
const MAX_RECURSION_DEPTH: usize = 20;

async fn list_recursive(
    current: &str,
    workspace_root: &Option<PathBuf>,
) -> Result<String, AgentError> {
    let mut result = Vec::new();
    // (directory path, depth)
    let mut stack: Vec<(String, usize)> = vec![(current.to_string(), 0)];
    // Track visited canonical paths to detect symlink cycles.
    let mut visited = std::collections::HashSet::new();

    // Seed visited set with the starting directory.
    if let Ok(canon) = tokio::fs::canonicalize(current).await {
        visited.insert(canon);
    }

    while let Some((dir, depth)) = stack.pop() {
        if depth >= MAX_RECURSION_DEPTH {
            continue;
        }

        let mut entries =
            tokio::fs::read_dir(&dir)
                .await
                .map_err(|e| AgentError::ToolExecution {
                    tool_name: "file_list".into(),
                    message: format!("Failed to read directory {}: {}", dir, e),
                })?;

        while let Some(entry) =
            entries
                .next_entry()
                .await
                .map_err(|e| AgentError::ToolExecution {
                    tool_name: "file_list".into(),
                    message: format!("Failed to read entry: {}", e),
                })?
        {
            let path = entry.path();
            let display = path.to_string_lossy().to_string();

            // Use file_type() which does NOT follow symlinks (like lstat).
            let ft = match entry.file_type().await {
                Ok(ft) => ft,
                Err(_) => continue,
            };

            if ft.is_symlink() {
                // For symlinks, resolve the target and validate it's within workspace.
                let target = match tokio::fs::canonicalize(&path).await {
                    Ok(t) => t,
                    Err(_) => {
                        // Dangling symlink — list it but don't traverse.
                        result.push(format!("{} -> [broken symlink]", display));
                        continue;
                    }
                };

                if let Some(root) = workspace_root {
                    if let Ok(canon_root) = root.canonicalize() {
                        if !target.starts_with(&canon_root) {
                            // Symlink points outside workspace — skip.
                            result.push(format!(
                                "{} -> [symlink outside workspace, skipped]",
                                display
                            ));
                            continue;
                        }
                    }
                }

                // Symlink target is within workspace. Check if it's a directory.
                let target_meta = match tokio::fs::metadata(&target).await {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if target_meta.is_dir() {
                    // Cycle detection: only traverse if we haven't visited this canonical path.
                    if visited.insert(target) {
                        result.push(format!("{}/", display));
                        stack.push((display, depth + 1));
                    } else {
                        result.push(format!("{} -> [symlink cycle, skipped]", display));
                    }
                } else {
                    result.push(display);
                }
            } else if ft.is_dir() {
                // Real directory (not a symlink). Cycle detection via canonical path.
                let canon = tokio::fs::canonicalize(&path).await.ok();
                let is_new = canon.is_none_or(|c| visited.insert(c));
                if is_new {
                    result.push(format!("{}/", display));
                    stack.push((display, depth + 1));
                }
            } else {
                result.push(display);
            }
        }
    }

    result.sort();
    Ok(result.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::tool_registry::Tool;
    use serde_json::json;
    use tempfile::TempDir;

    // ── validate_path unit tests ────────────────────────────────────

    #[test]
    fn test_validate_path_no_restriction() {
        let result = validate_path("/any/path/at/all", &None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_path_within_workspace() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().to_path_buf();
        let file = tmp.path().join("hello.txt");
        std::fs::write(&file, "hi").unwrap();

        let result = validate_path(file.to_str().unwrap(), &Some(workspace));
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_path_outside_workspace() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().to_path_buf();

        let result = validate_path("/etc/passwd", &Some(workspace));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("outside the workspace root"), "got: {msg}");
    }

    #[test]
    fn test_validate_path_traversal_blocked() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().to_path_buf();
        let traversal = format!("{}/../../etc/passwd", workspace.display());

        let result = validate_path(&traversal, &Some(workspace));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("outside the workspace root"), "got: {msg}");
    }

    #[test]
    fn test_validate_path_nonexistent_within_workspace() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().to_path_buf();
        let nonexistent = tmp.path().join("does_not_exist.txt");

        let result = validate_path(nonexistent.to_str().unwrap(), &Some(workspace));
        assert!(result.is_ok());
    }

    // ── Tool::execute integration tests ─────────────────────────────

    #[tokio::test]
    async fn test_file_read_blocked_outside_workspace() {
        let tmp = TempDir::new().unwrap();
        let tool = FileReadTool {
            workspace_root: Some(tmp.path().to_path_buf()),
        };
        let result = tool.execute(json!({"path": "/etc/hostname"})).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("outside the workspace root"), "got: {msg}");
    }

    #[tokio::test]
    async fn test_file_write_blocked_outside_workspace() {
        let tmp = TempDir::new().unwrap();
        let tool = FileWriteTool {
            workspace_root: Some(tmp.path().to_path_buf()),
        };
        let result = tool
            .execute(json!({"path": "/tmp/evil.txt", "content": "pwned"}))
            .await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("outside the workspace root"), "got: {msg}");
    }

    #[tokio::test]
    async fn test_file_list_blocked_outside_workspace() {
        let tmp = TempDir::new().unwrap();
        let tool = FileListTool {
            workspace_root: Some(tmp.path().to_path_buf()),
        };
        let result = tool.execute(json!({"path": "/etc"})).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("outside the workspace root"), "got: {msg}");
    }

    #[tokio::test]
    async fn test_file_read_allowed_within_workspace() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("test.txt");
        std::fs::write(&file, "hello world").unwrap();

        let tool = FileReadTool {
            workspace_root: Some(tmp.path().to_path_buf()),
        };
        let result = tool.execute(json!({"path": file.to_str().unwrap()})).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "hello world");
    }

    #[tokio::test]
    async fn test_recursive_list_skips_symlink_outside_workspace() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().to_path_buf();

        // Create a subdirectory and a file inside it.
        let subdir = workspace.join("subdir");
        std::fs::create_dir(&subdir).unwrap();
        std::fs::write(subdir.join("file.txt"), "ok").unwrap();

        // Create a symlink pointing outside the workspace (to /tmp or similar).
        let escape_link = workspace.join("escape");
        #[cfg(unix)]
        std::os::unix::fs::symlink("/usr", &escape_link).unwrap();

        let tool = FileListTool {
            workspace_root: Some(workspace.clone()),
        };
        let result = tool
            .execute(json!({"path": workspace.to_str().unwrap(), "recursive": true}))
            .await;
        assert!(result.is_ok());
        let listing = result.unwrap();

        // The symlink should be marked as skipped, not traversed.
        assert!(
            listing.contains("skipped"),
            "symlink outside workspace should be skipped, got:\n{listing}"
        );
        // /usr contents should NOT appear.
        assert!(
            !listing.contains("/usr/"),
            "should not traverse outside workspace, got:\n{listing}"
        );
    }

    #[tokio::test]
    async fn test_recursive_list_detects_symlink_cycle() {
        let tmp = TempDir::new().unwrap();
        let workspace = tmp.path().to_path_buf();

        let subdir = workspace.join("a");
        std::fs::create_dir(&subdir).unwrap();

        // Create a symlink cycle: a/loop -> workspace (which contains a/)
        let loop_link = subdir.join("loop");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&workspace, &loop_link).unwrap();

        let tool = FileListTool {
            workspace_root: Some(workspace.clone()),
        };
        let result = tool
            .execute(json!({"path": workspace.to_str().unwrap(), "recursive": true}))
            .await;
        // Should complete without infinite loop.
        assert!(result.is_ok());
    }
}
