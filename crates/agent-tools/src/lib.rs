pub mod shell_exec;
pub mod file_ops;
pub mod web_fetch;
pub mod python_exec;
pub mod sandbox;

use agent_core::tool_registry::ToolRegistry;
use agent_core::config::AppConfig;
use std::sync::Arc;

/// Register all built-in tools into the registry.
pub fn register_all(registry: &mut ToolRegistry, config: &AppConfig) {
    let executor = Arc::new(sandbox::SandboxExecutor::new(config));
    let workspace_root = config.sandbox.workspace_root.clone();

    registry.register(Arc::new(shell_exec::ShellExecTool::new(executor.clone())));
    registry.register(Arc::new(file_ops::FileReadTool { workspace_root: workspace_root.clone() }));
    registry.register(Arc::new(file_ops::FileWriteTool { workspace_root: workspace_root.clone() }));
    registry.register(Arc::new(file_ops::FileListTool { workspace_root }));
    registry.register(Arc::new(web_fetch::WebFetchTool::new()));
    registry.register(Arc::new(python_exec::PythonExecTool::new(executor)));
}
