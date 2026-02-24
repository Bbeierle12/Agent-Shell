//! Environment detection tool.
//!
//! Detects project types, runtime environments (Python venv, Node, Rust toolchain),
//! and git context for a given directory.

use agent_core::context::ContextLinker;
use agent_core::error::AgentError;
use agent_core::tool_registry::Tool;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Detected runtime environment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectedEnvironment {
    pub name: String,
    pub env_type: String,
    pub version: Option<String>,
    pub path: PathBuf,
}

/// Detect Python virtual environments.
fn detect_python_env(dir: &Path) -> Option<DetectedEnvironment> {
    let venv_paths = [".venv", "venv", ".env", "env"];

    for venv_name in venv_paths {
        let venv_path = dir.join(venv_name);
        let pyvenv_cfg = venv_path.join("pyvenv.cfg");

        if pyvenv_cfg.exists() {
            let version = parse_pyvenv_version(&pyvenv_cfg);

            return Some(DetectedEnvironment {
                name: format!("Python ({})", venv_name),
                env_type: "python-venv".to_string(),
                version,
                path: venv_path,
            });
        }
    }

    // Check for conda-meta.
    let conda_meta = dir.join("conda-meta");
    if conda_meta.is_dir() {
        return Some(DetectedEnvironment {
            name: "Python (conda)".to_string(),
            env_type: "python-conda".to_string(),
            version: detect_conda_python(&conda_meta),
            path: dir.to_path_buf(),
        });
    }

    None
}

/// Parse Python version from pyvenv.cfg.
fn parse_pyvenv_version(cfg_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(cfg_path).ok()?;
    for line in content.lines() {
        if line.starts_with("version") {
            return line.split('=').nth(1).map(|v| v.trim().to_string());
        }
    }
    None
}

/// Get Python version from conda metadata.
fn detect_conda_python(conda_meta: &Path) -> Option<String> {
    if let Ok(entries) = std::fs::read_dir(conda_meta) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("python-") && name_str.ends_with(".json") {
                return name_str
                    .strip_prefix("python-")
                    .and_then(|s| s.split('-').next())
                    .map(String::from);
            }
        }
    }
    None
}

/// Detect Node.js environment.
fn detect_node_env(dir: &Path) -> Option<DetectedEnvironment> {
    let package_json = dir.join("package.json");
    if !package_json.exists() {
        return None;
    }

    let version = detect_node_version(dir);
    let manager = detect_node_manager(dir);

    Some(DetectedEnvironment {
        name: format!("Node.js ({})", manager),
        env_type: "node".to_string(),
        version,
        path: dir.to_path_buf(),
    })
}

/// Detect Node.js version from various sources.
fn detect_node_version(dir: &Path) -> Option<String> {
    // Check .nvmrc.
    let nvmrc = dir.join(".nvmrc");
    if nvmrc.exists() {
        if let Ok(content) = std::fs::read_to_string(&nvmrc) {
            return Some(content.trim().to_string());
        }
    }

    // Check .node-version.
    let node_version = dir.join(".node-version");
    if node_version.exists() {
        if let Ok(content) = std::fs::read_to_string(&node_version) {
            return Some(content.trim().to_string());
        }
    }

    // Check package.json engines.
    let package_json = dir.join("package.json");
    if package_json.exists() {
        if let Ok(content) = std::fs::read_to_string(&package_json) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(node) = json.pointer("/engines/node") {
                    return node.as_str().map(String::from);
                }
            }
        }
    }

    None
}

/// Detect which Node manager is being used.
fn detect_node_manager(dir: &Path) -> &'static str {
    let package_json = dir.join("package.json");
    if package_json.exists() {
        if let Ok(content) = std::fs::read_to_string(&package_json) {
            if content.contains("\"volta\"") {
                return "volta";
            }
        }
    }

    if dir.join(".nvmrc").exists() {
        return "nvm";
    }

    if dir.join(".node-version").exists() {
        return "fnm";
    }

    "direct"
}

/// Detect Rust toolchain.
fn detect_rust_env(dir: &Path) -> Option<DetectedEnvironment> {
    let rust_toolchain = dir.join("rust-toolchain.toml");
    let rust_toolchain_legacy = dir.join("rust-toolchain");

    let version = if rust_toolchain.exists() {
        std::fs::read_to_string(&rust_toolchain)
            .ok()
            .and_then(|content| {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("channel") {
                        return trimmed
                            .split('=')
                            .nth(1)
                            .map(|v| v.trim().trim_matches('"').to_string());
                    }
                }
                None
            })
    } else if rust_toolchain_legacy.exists() {
        std::fs::read_to_string(&rust_toolchain_legacy)
            .ok()
            .map(|s| s.trim().to_string())
    } else if dir.join("Cargo.toml").exists() {
        // Just note Rust is present without a specific toolchain pinned.
        Some("default".to_string())
    } else {
        return None;
    };

    Some(DetectedEnvironment {
        name: "Rust".to_string(),
        env_type: "rust".to_string(),
        version,
        path: dir.to_path_buf(),
    })
}

/// Detect Go environment.
fn detect_go_env(dir: &Path) -> Option<DetectedEnvironment> {
    if !dir.join("go.mod").exists() {
        return None;
    }

    // Try to extract Go version from go.mod.
    let version = std::fs::read_to_string(dir.join("go.mod"))
        .ok()
        .and_then(|content| {
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("go ") {
                    return Some(trimmed.strip_prefix("go ")?.trim().to_string());
                }
            }
            None
        });

    Some(DetectedEnvironment {
        name: "Go".to_string(),
        env_type: "go".to_string(),
        version,
        path: dir.to_path_buf(),
    })
}

/// Detect all environments in a directory.
pub fn detect_environments(dir: &Path) -> Vec<DetectedEnvironment> {
    let mut envs = Vec::new();

    if let Some(env) = detect_python_env(dir) {
        envs.push(env);
    }
    if let Some(env) = detect_node_env(dir) {
        envs.push(env);
    }
    if let Some(env) = detect_rust_env(dir) {
        envs.push(env);
    }
    if let Some(env) = detect_go_env(dir) {
        envs.push(env);
    }

    envs
}

/// Tool that detects project type, runtime environments, and git context.
#[derive(Default)]
pub struct EnvDetectTool;

impl EnvDetectTool {
    pub fn new() -> Self {
        Self
    }

    fn err(msg: impl Into<String>) -> AgentError {
        AgentError::ToolExecution {
            tool_name: "env_detect".into(),
            message: msg.into(),
        }
    }
}

#[async_trait]
impl Tool for EnvDetectTool {
    fn name(&self) -> &str {
        "env_detect"
    }

    fn description(&self) -> &str {
        "Detect project type, runtime environments, and git context for a directory."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "directory": {
                    "type": "string",
                    "description": "Directory to analyze (defaults to current working directory)"
                }
            }
        })
    }

    async fn execute(&self, args: Value) -> Result<String, AgentError> {
        let dir = args
            .get("directory")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

        if !dir.is_dir() {
            return Err(Self::err(format!("Not a directory: {}", dir.display())));
        }

        let mut output = String::new();

        // Detect project.
        let mut linker = ContextLinker::new();
        if let Some(project) = linker.detect_project(&dir) {
            output.push_str(&format!("**Project:** {}\n", project.name));
            output.push_str(&format!(
                "**Type:** {}\n",
                project.primary_type().display_name()
            ));
            output.push_str(&format!("**Path:** {}\n", project.path.display()));
            if let Some(remote) = &project.git_remote {
                output.push_str(&format!("**Git Remote:** {}\n", remote));
            }
            if let Some(branch) = &project.git_branch {
                output.push_str(&format!("**Git Branch:** {}\n", branch));
            }
        } else {
            output.push_str("**Project:** (none detected)\n");
        }

        // Detect git context.
        if let Some(git) = ContextLinker::get_git_context(&dir) {
            output.push_str("\n**Git:**\n");
            if let Some(branch) = &git.branch {
                output.push_str(&format!("  Branch: {}\n", branch));
            }
            if let Some(head) = &git.head_short {
                output.push_str(&format!("  HEAD: {}\n", head));
            }
            output.push_str(&format!(
                "  Dirty: {}\n",
                if git.is_dirty { "yes" } else { "no" }
            ));
            output.push_str(&format!("  Root: {}\n", git.repo_root.display()));
        }

        // Detect runtime environments.
        let envs = detect_environments(&dir);
        if envs.is_empty() {
            output.push_str("\n**Environments:** (none detected)\n");
        } else {
            output.push_str(&format!("\n**Environments ({}):**\n", envs.len()));
            for env in &envs {
                output.push_str(&format!("  - {}", env.name));
                if let Some(version) = &env.version {
                    output.push_str(&format!(" v{}", version));
                }
                output.push('\n');
            }
        }

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_detect_python_venv() {
        let dir = TempDir::new().unwrap();
        let venv_dir = dir.path().join(".venv");
        std::fs::create_dir_all(&venv_dir).unwrap();
        std::fs::write(
            venv_dir.join("pyvenv.cfg"),
            "home = /usr/bin\nversion = 3.12.1\n",
        )
        .unwrap();

        let env = detect_python_env(dir.path());
        assert!(env.is_some());
        let env = env.unwrap();
        assert_eq!(env.env_type, "python-venv");
        assert_eq!(env.version, Some("3.12.1".to_string()));
    }

    #[test]
    fn test_detect_node_env() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();

        let env = detect_node_env(dir.path());
        assert!(env.is_some());
        assert_eq!(env.unwrap().env_type, "node");
    }

    #[test]
    fn test_detect_node_version_nvmrc() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();
        std::fs::write(dir.path().join(".nvmrc"), "20.11.0\n").unwrap();

        let env = detect_node_env(dir.path()).unwrap();
        assert_eq!(env.version, Some("20.11.0".to_string()));
    }

    #[test]
    fn test_detect_rust_env() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();

        let env = detect_rust_env(dir.path());
        assert!(env.is_some());
        assert_eq!(env.unwrap().env_type, "rust");
    }

    #[test]
    fn test_detect_rust_toolchain() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(
            dir.path().join("rust-toolchain.toml"),
            "[toolchain]\nchannel = \"1.78\"\n",
        )
        .unwrap();

        let env = detect_rust_env(dir.path()).unwrap();
        assert_eq!(env.version, Some("1.78".to_string()));
    }

    #[test]
    fn test_detect_go_env() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("go.mod"),
            "module example.com/foo\n\ngo 1.22\n",
        )
        .unwrap();

        let env = detect_go_env(dir.path());
        assert!(env.is_some());
        let env = env.unwrap();
        assert_eq!(env.env_type, "go");
        assert_eq!(env.version, Some("1.22".to_string()));
    }

    #[test]
    fn test_detect_no_environments() {
        let dir = TempDir::new().unwrap();
        let envs = detect_environments(dir.path());
        assert!(envs.is_empty());
    }

    #[test]
    fn test_detect_multiple_environments() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(dir.path().join("package.json"), "{}").unwrap();

        let envs = detect_environments(dir.path());
        assert_eq!(envs.len(), 2);
    }

    #[tokio::test]
    async fn test_env_detect_tool() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();

        let tool = EnvDetectTool::new();
        let result = tool
            .execute(serde_json::json!({
                "directory": dir.path().to_string_lossy()
            }))
            .await
            .unwrap();

        assert!(result.contains("Rust"));
    }

    #[tokio::test]
    async fn test_env_detect_tool_invalid_dir() {
        let tool = EnvDetectTool::new();
        let result = tool
            .execute(serde_json::json!({
                "directory": "/nonexistent/path/xyz"
            }))
            .await;

        assert!(result.is_err());
    }
}
