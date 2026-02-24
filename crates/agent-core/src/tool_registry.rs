use crate::error::AgentError;
use crate::types::{ToolOutput, ToolSchema};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// Trait that all tools must implement.
#[async_trait]
pub trait Tool: Send + Sync {
    /// The unique name of this tool (used in function calling).
    fn name(&self) -> &str;

    /// Human-readable description of what the tool does.
    fn description(&self) -> &str;

    /// JSON Schema describing the tool's parameters.
    fn parameters_schema(&self) -> Value;

    /// Execute the tool with the given arguments.
    async fn execute(&self, args: Value) -> Result<String, AgentError>;
}

/// Central registry for all available tools.
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool. Overwrites any existing tool with the same name.
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.name().to_string();
        tracing::debug!("Registered tool: {}", name);
        self.tools.insert(name, tool);
    }

    /// Unregister a tool by name.
    pub fn unregister(&mut self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.remove(name)
    }

    /// Get a tool by name.
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    /// List all registered tool names.
    pub fn list_names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Get the tool schemas for all registered tools, suitable for sending to the model.
    /// Optionally filtered by an allowlist and denylist.
    pub fn schemas(&self, allowlist: Option<&[String]>, denylist: &[String]) -> Vec<ToolSchema> {
        self.tools
            .values()
            .filter(|t| {
                let name = t.name().to_string();
                if denylist.contains(&name) {
                    return false;
                }
                match allowlist {
                    Some(allow) => allow.contains(&name),
                    None => true,
                }
            })
            .map(|t| ToolSchema {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters_schema(),
            })
            .collect()
    }

    /// Execute a tool by name with the given arguments.
    pub async fn execute(&self, tool_name: &str, tool_call_id: &str, args: Value) -> ToolOutput {
        match self.tools.get(tool_name) {
            Some(tool) => match tool.execute(args).await {
                Ok(content) => ToolOutput {
                    tool_call_id: tool_call_id.to_string(),
                    content,
                    is_error: false,
                },
                Err(e) => ToolOutput {
                    tool_call_id: tool_call_id.to_string(),
                    content: format!("Error: {}", e),
                    is_error: true,
                },
            },
            None => ToolOutput {
                tool_call_id: tool_call_id.to_string(),
                content: format!("Tool not found: {}", tool_name),
                is_error: true,
            },
        }
    }

    /// Number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert tool schemas to the OpenAI function calling format.
pub fn schemas_to_openai_tools(schemas: &[ToolSchema]) -> Vec<Value> {
    schemas
        .iter()
        .map(|s| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": s.name,
                    "description": s.description,
                    "parameters": s.parameters,
                }
            })
        })
        .collect()
}
