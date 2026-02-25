use crate::config::AppConfig;
use crate::error::AgentError;
use crate::provider::{ProviderChain, RequestError, ResolvedProvider};
use crate::tool_registry::ToolRegistry;
use crate::types::{AgentEvent, Message, Role, ToolCall, ToolOutput, ToolSchema};

use async_openai::config::OpenAIConfig;
use async_openai::types::{
    ChatCompletionMessageToolCall, ChatCompletionRequestAssistantMessageArgs,
    ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
    ChatCompletionRequestToolMessageArgs, ChatCompletionRequestUserMessageArgs,
    ChatCompletionToolArgs, ChatCompletionToolType, CreateChatCompletionRequestArgs,
    FunctionObjectArgs,
};
use async_openai::Client;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Maximum number of tool-calling iterations before we force a text response.
const MAX_TOOL_ITERATIONS: usize = 20;

/// The core agent loop — orchestrates LLM calls and tool execution.
pub struct AgentLoop {
    provider_chain: ProviderChain,
    config: AppConfig,
    tool_registry: Arc<ToolRegistry>,
}

impl AgentLoop {
    /// Create a new agent loop with provider failover support.
    pub fn new(config: AppConfig, tool_registry: Arc<ToolRegistry>) -> Result<Self, AgentError> {
        let provider_chain = ProviderChain::from_config(&config)?;
        Ok(Self {
            provider_chain,
            config,
            tool_registry,
        })
    }

    /// Run the agent for a single user turn. Takes the full message history and
    /// returns the final assistant message, sending streaming events to the channel.
    pub async fn run(
        &self,
        messages: &[Message],
        session_tool_allowlist: Option<&[String]>,
        session_tool_denylist: &[String],
        event_tx: mpsc::UnboundedSender<AgentEvent>,
    ) -> Result<Message, AgentError> {
        let tool_schemas = self
            .tool_registry
            .schemas(session_tool_allowlist, session_tool_denylist);

        // Build OpenAI tool definitions once (they don't change between iterations).
        let openai_tools = build_chat_tools(&tool_schemas)?;

        // Build the set of allowed tool names for runtime policy enforcement.
        let allowed_tools: HashSet<String> = tool_schemas.iter().map(|s| s.name.clone()).collect();

        // Build the running message list (we'll extend it with tool results).
        let mut running_messages = self.build_openai_messages(messages)?;
        let mut iteration = 0;

        loop {
            iteration += 1;
            if iteration > MAX_TOOL_ITERATIONS {
                warn!(
                    "Hit max tool iterations ({}), forcing text response",
                    MAX_TOOL_ITERATIONS
                );
                break;
            }

            debug!("Agent loop iteration {}", iteration);

            // Snapshot messages and tools for the closure.
            let msgs_snapshot = running_messages.clone();
            let tools_snapshot = openai_tools.clone();

            // Use provider chain with automatic failover for the LLM call.
            let response = self
                .provider_chain
                .request_with_failover(None, |provider| {
                    let msgs = msgs_snapshot.clone();
                    let tools = tools_snapshot.clone();
                    async move { send_completion_request(provider, msgs, tools).await }
                })
                .await?;

            let choice = response
                .choices
                .first()
                .ok_or_else(|| AgentError::Provider("No choices in response".into()))?;

            let assistant_msg = &choice.message;
            let content = assistant_msg.content.clone().unwrap_or_default();

            // Check for tool calls.
            if let Some(tool_calls) = &assistant_msg.tool_calls {
                if !tool_calls.is_empty() {
                    // Send content tokens if any.
                    if !content.is_empty() {
                        let _ = event_tx.send(AgentEvent::ContentChunk(content.clone()));
                    }

                    // Add assistant's message with tool calls to running history.
                    let tc_openai: Vec<ChatCompletionMessageToolCall> = tool_calls.clone();
                    let assistant_openai = ChatCompletionRequestAssistantMessageArgs::default()
                        .content(&*content)
                        .tool_calls(tc_openai.clone())
                        .build()
                        .map_err(|e| AgentError::Provider(e.to_string()))?;
                    running_messages
                        .push(ChatCompletionRequestMessage::Assistant(assistant_openai));

                    // Execute each tool call.
                    let our_tool_calls: Vec<ToolCall> = tool_calls
                        .iter()
                        .map(|tc| ToolCall {
                            id: tc.id.clone(),
                            name: tc.function.name.clone(),
                            arguments: tc.function.arguments.clone(),
                        })
                        .collect();

                    let mut tool_outputs = Vec::new();
                    for tc in &our_tool_calls {
                        let _ = event_tx.send(AgentEvent::ToolCallStart {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                        });

                        // Policy enforcement: reject tools not in the allowed set.
                        let output = if !allowed_tools.contains(&tc.name) {
                            ToolOutput {
                                tool_call_id: tc.id.clone(),
                                content: format!("Tool not allowed: {}", tc.name),
                                is_error: true,
                            }
                        } else {
                            // Parse arguments, returning an error to the model on failure.
                            let args: serde_json::Value = match serde_json::from_str(&tc.arguments)
                            {
                                Ok(v) => v,
                                Err(e) => {
                                    let err_output = ToolOutput {
                                        tool_call_id: tc.id.clone(),
                                        content: format!("Invalid JSON arguments: {}", e),
                                        is_error: true,
                                    };
                                    let _ =
                                        event_tx.send(AgentEvent::ToolResult(err_output.clone()));
                                    let tool_msg = ChatCompletionRequestToolMessageArgs::default()
                                        .tool_call_id(&tc.id)
                                        .content(&*err_output.content)
                                        .build()
                                        .map_err(|e| AgentError::Provider(e.to_string()))?;
                                    running_messages
                                        .push(ChatCompletionRequestMessage::Tool(tool_msg));
                                    tool_outputs.push(err_output);
                                    continue;
                                }
                            };
                            self.tool_registry.execute(&tc.name, &tc.id, args).await
                        };

                        let _ = event_tx.send(AgentEvent::ToolResult(output.clone()));

                        // Add tool result to running messages.
                        let tool_msg = ChatCompletionRequestToolMessageArgs::default()
                            .tool_call_id(&tc.id)
                            .content(&*output.content)
                            .build()
                            .map_err(|e| AgentError::Provider(e.to_string()))?;
                        running_messages.push(ChatCompletionRequestMessage::Tool(tool_msg));

                        tool_outputs.push(output);
                    }

                    // Continue the loop — the model needs to process tool results.
                    continue;
                }
            }

            // No tool calls — this is the final text response.
            if !content.is_empty() {
                let _ = event_tx.send(AgentEvent::ContentChunk(content.clone()));
            }

            let final_message = Message::assistant(&content);
            let _ = event_tx.send(AgentEvent::Done(final_message.clone()));
            return Ok(final_message);
        }

        // If we hit max iterations, return whatever we have.
        let fallback = Message::assistant("[Agent reached maximum tool iterations]");
        let _ = event_tx.send(AgentEvent::Done(fallback.clone()));
        Ok(fallback)
    }

    /// Convert our Message types to async-openai request messages.
    fn build_openai_messages(
        &self,
        messages: &[Message],
    ) -> Result<Vec<ChatCompletionRequestMessage>, AgentError> {
        let mut result = Vec::new();

        // Inject system prompt if configured and not already present.
        let has_system = messages.iter().any(|m| m.role == Role::System);
        if !has_system {
            if let Some(sys_prompt) = &self.config.system_prompt {
                let sys_msg = ChatCompletionRequestSystemMessageArgs::default()
                    .content(sys_prompt.as_str())
                    .build()
                    .map_err(|e| AgentError::Provider(e.to_string()))?;
                result.push(ChatCompletionRequestMessage::System(sys_msg));
            }
        }

        for msg in messages {
            match msg.role {
                Role::System => {
                    let m = ChatCompletionRequestSystemMessageArgs::default()
                        .content(msg.content.as_str())
                        .build()
                        .map_err(|e| AgentError::Provider(e.to_string()))?;
                    result.push(ChatCompletionRequestMessage::System(m));
                }
                Role::User => {
                    let m = ChatCompletionRequestUserMessageArgs::default()
                        .content(msg.content.as_str())
                        .build()
                        .map_err(|e| AgentError::Provider(e.to_string()))?;
                    result.push(ChatCompletionRequestMessage::User(m));
                }
                Role::Assistant => {
                    let mut builder = ChatCompletionRequestAssistantMessageArgs::default();
                    builder.content(msg.content.as_str());
                    if let Some(tool_calls) = &msg.tool_calls {
                        let tc_openai: Vec<ChatCompletionMessageToolCall> = tool_calls
                            .iter()
                            .map(|tc| ChatCompletionMessageToolCall {
                                id: tc.id.clone(),
                                r#type: ChatCompletionToolType::Function,
                                function: async_openai::types::FunctionCall {
                                    name: tc.name.clone(),
                                    arguments: tc.arguments.clone(),
                                },
                            })
                            .collect();
                        builder.tool_calls(tc_openai);
                    }
                    let m = builder
                        .build()
                        .map_err(|e| AgentError::Provider(e.to_string()))?;
                    result.push(ChatCompletionRequestMessage::Assistant(m));
                }
                Role::Tool => {
                    let m = ChatCompletionRequestToolMessageArgs::default()
                        .tool_call_id(msg.tool_call_id.as_deref().unwrap_or(""))
                        .content(msg.content.as_str())
                        .build()
                        .map_err(|e| AgentError::Provider(e.to_string()))?;
                    result.push(ChatCompletionRequestMessage::Tool(m));
                }
            }
        }

        Ok(result)
    }
}

/// Build OpenAI-format tool definitions from our tool schemas.
fn build_chat_tools(
    schemas: &[ToolSchema],
) -> Result<Vec<async_openai::types::ChatCompletionTool>, AgentError> {
    schemas
        .iter()
        .map(|s| {
            let func = FunctionObjectArgs::default()
                .name(&s.name)
                .description(&s.description)
                .parameters(s.parameters.clone())
                .build()
                .map_err(|e| AgentError::Schema(format!("function '{}': {}", s.name, e)))?;
            ChatCompletionToolArgs::default()
                .r#type(ChatCompletionToolType::Function)
                .function(func)
                .build()
                .map_err(|e| AgentError::Schema(format!("tool '{}': {}", s.name, e)))
        })
        .collect()
}

/// Send a chat completion request to a specific provider.
async fn send_completion_request(
    provider: ResolvedProvider,
    messages: Vec<ChatCompletionRequestMessage>,
    tools: Vec<async_openai::types::ChatCompletionTool>,
) -> Result<async_openai::types::CreateChatCompletionResponse, RequestError> {
    let openai_config = OpenAIConfig::new()
        .with_api_base(&provider.api_base)
        .with_api_key(provider.api_key.as_deref().unwrap_or("not-needed"));
    let client = Client::with_config(openai_config);

    let mut request_builder = CreateChatCompletionRequestArgs::default();
    request_builder
        .model(&provider.model)
        .messages(messages)
        .temperature(provider.temperature)
        .max_completion_tokens(provider.max_tokens);

    if !tools.is_empty() {
        request_builder.tools(tools);
    }

    let request = request_builder
        .build()
        .map_err(|e| RequestError::Permanent(format!("Failed to build request: {}", e)))?;

    let timeout_duration = std::time::Duration::from_secs(provider.timeout_secs);
    match tokio::time::timeout(timeout_duration, client.chat().create(request)).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(e)) => classify_provider_error(e),
        Err(_) => Err(RequestError::Transient(format!(
            "Request timed out after {}s",
            provider.timeout_secs
        ))),
    }
}

/// Classify an async-openai error for failover decisions.
///
/// Auth/config errors (4xx patterns) are Permanent — do not failover.
/// Everything else (5xx, network, timeout) is Transient — try next provider.
fn classify_provider_error<T>(err: async_openai::error::OpenAIError) -> Result<T, RequestError> {
    let msg = err.to_string();
    let lower = msg.to_lowercase();

    let permanent_patterns = [
        "invalid_api_key",
        "authentication",
        "unauthorized",
        "invalid_request",
        "model_not_found",
        "permission",
    ];

    if permanent_patterns.iter().any(|p| lower.contains(p)) {
        Err(RequestError::Permanent(msg))
    } else {
        Err(RequestError::Transient(msg))
    }
}
