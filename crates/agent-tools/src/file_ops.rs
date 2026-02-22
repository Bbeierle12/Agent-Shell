use agent_core::error::AgentError;
use agent_core::tool_registry::Tool;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::Path;

// ── file_read ──────────────────────────────────────────────────────────

pub struct FileReadTool;

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
        let args: Args = serde_json::from_value(args).map_err(|e| {
            AgentError::ToolExecution {
                tool_name: "file_read".into(),
                message: format!("Invalid arguments: {}", e),
            }
        })?;

        let content = tokio::fs::read_to_string(&args.path).await.map_err(|e| {
            AgentError::ToolExecution {
                tool_name: "file_read".into(),
                message: format!("Failed to read {}: {}", args.path, e),
            }
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

pub struct FileWriteTool;

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
        let args: Args = serde_json::from_value(args).map_err(|e| {
            AgentError::ToolExecution {
                tool_name: "file_write".into(),
                message: format!("Invalid arguments: {}", e),
            }
        })?;

        let path = Path::new(&args.path);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                AgentError::ToolExecution {
                    tool_name: "file_write".into(),
                    message: format!("Failed to create directories: {}", e),
                }
            })?;
        }

        if args.append {
            use tokio::io::AsyncWriteExt;
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&args.path)
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
            tokio::fs::write(&args.path, &args.content).await.map_err(|e| {
                AgentError::ToolExecution {
                    tool_name: "file_write".into(),
                    message: format!("Failed to write {}: {}", args.path, e),
                }
            })?;
        }

        let bytes = args.content.len();
        Ok(format!("Wrote {} bytes to {}", bytes, args.path))
    }
}

// ── file_list ──────────────────────────────────────────────────────────

pub struct FileListTool;

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

        let args: Args = serde_json::from_value(args).map_err(|e| {
            AgentError::ToolExecution {
                tool_name: "file_list".into(),
                message: format!("Invalid arguments: {}", e),
            }
        })?;

        if args.recursive {
            list_recursive(&args.path, &args.path).await
        } else {
            list_flat(&args.path).await
        }
    }
}

async fn list_flat(path: &str) -> Result<String, AgentError> {
    let mut entries = tokio::fs::read_dir(path).await.map_err(|e| {
        AgentError::ToolExecution {
            tool_name: "file_list".into(),
            message: format!("Failed to read directory {}: {}", path, e),
        }
    })?;

    let mut names = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|e| {
        AgentError::ToolExecution {
            tool_name: "file_list".into(),
            message: format!("Failed to read entry: {}", e),
        }
    })? {
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

async fn list_recursive(_base: &str, current: &str) -> Result<String, AgentError> {
    let mut result = Vec::new();
    let mut stack = vec![current.to_string()];

    while let Some(dir) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&dir).await.map_err(|e| {
            AgentError::ToolExecution {
                tool_name: "file_list".into(),
                message: format!("Failed to read directory {}: {}", dir, e),
            }
        })?;

        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            AgentError::ToolExecution {
                tool_name: "file_list".into(),
                message: format!("Failed to read entry: {}", e),
            }
        })? {
            let path = entry.path();
            let display = path.to_string_lossy().to_string();
            let meta = entry.metadata().await.ok();
            if meta.map(|m| m.is_dir()).unwrap_or(false) {
                result.push(format!("{}/", display));
                stack.push(display);
            } else {
                result.push(display);
            }
        }
    }

    result.sort();
    Ok(result.join("\n"))
}
