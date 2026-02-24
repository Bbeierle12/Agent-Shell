pub mod env_detect;
pub mod file_ops;
pub mod python_exec;
pub mod sandbox;
pub mod shell_exec;
pub mod skill_load;
pub mod web_fetch;

use agent_core::config::AppConfig;
use agent_core::tool_registry::ToolRegistry;
use agent_skills::SkillIndexer;
use std::sync::Arc;

/// Register all built-in tools into the registry.
pub fn register_all(
    registry: &mut ToolRegistry,
    config: &AppConfig,
    skill_indexer: Option<Arc<SkillIndexer>>,
) {
    let executor = Arc::new(sandbox::SandboxExecutor::new(config));
    let workspace_root = config.sandbox.workspace_root.clone();

    registry.register(Arc::new(shell_exec::ShellExecTool::new(executor.clone())));
    registry.register(Arc::new(file_ops::FileReadTool {
        workspace_root: workspace_root.clone(),
    }));
    registry.register(Arc::new(file_ops::FileWriteTool {
        workspace_root: workspace_root.clone(),
    }));
    registry.register(Arc::new(file_ops::FileListTool { workspace_root }));
    registry.register(Arc::new(web_fetch::WebFetchTool::new()));
    registry.register(Arc::new(python_exec::PythonExecTool::new(executor)));
    registry.register(Arc::new(env_detect::EnvDetectTool::new()));

    // Register skill_load tool if a skill indexer is available.
    if let Some(indexer) = skill_indexer {
        registry.register(Arc::new(skill_load::SkillLoadTool::new(indexer)));
    }
}
