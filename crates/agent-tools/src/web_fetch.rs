use agent_core::error::AgentError;
use agent_core::tool_registry::Tool;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

/// Fetch a web page and return its text content.
pub struct WebFetchTool {
    client: reqwest::Client,
}

impl WebFetchTool {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("agent-shell/0.1")
            .build()
            .unwrap_or_default();
        Self { client }
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a web page by URL and return its text content. \
         Useful for reading documentation, APIs, or any publicly accessible web page."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch"
                },
                "max_length": {
                    "type": "integer",
                    "description": "Maximum characters to return. Default: 10000"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: Value) -> Result<String, AgentError> {
        #[derive(Deserialize)]
        struct Args {
            url: String,
            #[serde(default = "default_max")]
            max_length: usize,
        }
        fn default_max() -> usize {
            10000
        }

        let args: Args = serde_json::from_value(args).map_err(|e| {
            AgentError::ToolExecution {
                tool_name: "web_fetch".into(),
                message: format!("Invalid arguments: {}", e),
            }
        })?;

        let response = self.client.get(&args.url).send().await.map_err(|e| {
            AgentError::ToolExecution {
                tool_name: "web_fetch".into(),
                message: format!("Request failed: {}", e),
            }
        })?;

        let status = response.status();
        let body = response.text().await.map_err(|e| {
            AgentError::ToolExecution {
                tool_name: "web_fetch".into(),
                message: format!("Failed to read response body: {}", e),
            }
        })?;

        let truncated = if body.len() > args.max_length {
            format!(
                "{}... [truncated, {} total chars]",
                &body[..args.max_length],
                body.len()
            )
        } else {
            body
        };

        Ok(format!("HTTP {}\n\n{}", status.as_u16(), truncated))
    }
}
