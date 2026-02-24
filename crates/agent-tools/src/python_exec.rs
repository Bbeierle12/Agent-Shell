use crate::sandbox::SandboxExecutor;
use agent_core::error::AgentError;
use agent_core::tool_registry::Tool;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

/// Execute Python code via the sandbox.
pub struct PythonExecTool {
    executor: Arc<SandboxExecutor>,
}

impl PythonExecTool {
    pub fn new(executor: Arc<SandboxExecutor>) -> Self {
        Self { executor }
    }
}

#[async_trait]
impl Tool for PythonExecTool {
    fn name(&self) -> &str {
        "python_exec"
    }

    fn description(&self) -> &str {
        "Execute Python code and return the output. Use this for calculations, data processing, \
         file manipulation, or any task that benefits from running code. The code runs in a \
         Python 3 environment."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "The Python code to execute"
                }
            },
            "required": ["code"]
        })
    }

    async fn execute(&self, args: Value) -> Result<String, AgentError> {
        #[derive(Deserialize)]
        struct Args {
            code: String,
        }
        let args: Args = serde_json::from_value(args).map_err(|e| AgentError::ToolExecution {
            tool_name: "python_exec".into(),
            message: format!("Invalid arguments: {}", e),
        })?;

        let result = self.executor.exec_python(&args.code).await?;
        Ok(result.to_display_string())
    }
}
