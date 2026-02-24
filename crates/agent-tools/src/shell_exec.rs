use crate::sandbox::SandboxExecutor;
use agent_core::error::AgentError;
use agent_core::tool_registry::Tool;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

/// Execute shell commands via the sandbox.
pub struct ShellExecTool {
    executor: Arc<SandboxExecutor>,
}

impl ShellExecTool {
    pub fn new(executor: Arc<SandboxExecutor>) -> Self {
        Self { executor }
    }
}

#[async_trait]
impl Tool for ShellExecTool {
    fn name(&self) -> &str {
        "shell_exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return its output. Use this for running system commands, \
         installing packages, checking system state, etc."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value) -> Result<String, AgentError> {
        #[derive(Deserialize)]
        struct Args {
            command: String,
        }
        let args: Args = serde_json::from_value(args).map_err(|e| AgentError::ToolExecution {
            tool_name: "shell_exec".into(),
            message: format!("Invalid arguments: {}", e),
        })?;

        let result = self.executor.exec_shell(&args.command).await?;
        Ok(result.to_display_string())
    }
}
