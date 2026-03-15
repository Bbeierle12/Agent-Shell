use crate::config::AppConfig;
use crate::error::AgentError;
use crate::provider::{ProviderChain, RequestError, ResolvedProvider};
use crate::tool_loop::ToolLoopConfig;
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
use futures::StreamExt;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tracing::{debug, warn};

/// Result of a single agent turn, containing the final response and all
/// intermediate messages (assistant tool-call messages + tool result messages)
/// that should be persisted for complete conversation history.
#[derive(Debug, Clone)]
pub struct AgentTurnResult {
    /// All messages generated during this turn, in order. Includes:
    /// - Assistant messages with tool calls
    /// - Tool result messages
    /// - The final assistant text response (last element)
    pub messages: Vec<Message>,
}

impl AgentTurnResult {
    /// Get the final assistant message (the last message in the sequence).
    pub fn final_message(&self) -> &Message {
        self.messages.last().expect("AgentTurnResult must have at least one message")
    }
}

/// The core agent loop — orchestrates LLM calls and tool execution.
///
/// Uses [`ToolLoopConfig`] from `tool_loop.rs` for iteration limits, per-turn
/// tool call caps, and wall-clock timeout — replacing the previous hardcoded
/// `MAX_TOOL_ITERATIONS` constant.
pub struct AgentLoop {
    provider_chain: ProviderChain,
    config: AppConfig,
    tool_registry: Arc<ToolRegistry>,
    loop_config: ToolLoopConfig,
}

impl AgentLoop {
    /// Create a new agent loop with provider failover support.
    pub fn new(config: AppConfig, tool_registry: Arc<ToolRegistry>) -> Result<Self, AgentError> {
        let provider_chain = ProviderChain::from_config(&config)?;
        let loop_config = ToolLoopConfig::default();
        Ok(Self {
            provider_chain,
            config,
            tool_registry,
            loop_config,
        })
    }

    /// Create a new agent loop with custom tool loop configuration.
    pub fn with_loop_config(
        config: AppConfig,
        tool_registry: Arc<ToolRegistry>,
        loop_config: ToolLoopConfig,
    ) -> Result<Self, AgentError> {
        loop_config.validate()?;
        let provider_chain = ProviderChain::from_config(&config)?;
        Ok(Self {
            provider_chain,
            config,
            tool_registry,
            loop_config,
        })
    }

    /// Run the agent for a single user turn. Takes the full message history and
    /// returns all generated messages (assistant messages with tool calls, tool
    /// result messages, and the final assistant response).
    ///
    /// Uses true SSE streaming — content chunks are emitted as they arrive from
    /// the LLM, rather than buffering the entire response.
    pub async fn run(
        &self,
        messages: &[Message],
        session_tool_allowlist: Option<&[String]>,
        session_tool_denylist: &[String],
        event_tx: mpsc::UnboundedSender<AgentEvent>,
    ) -> Result<AgentTurnResult, AgentError> {
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
        let loop_start = std::time::Instant::now();
        // Track all messages generated during this turn for session persistence.
        let mut turn_messages: Vec<Message> = Vec::new();

        loop {
            iteration += 1;
            if iteration > self.loop_config.max_iterations {
                warn!(
                    "Hit max tool iterations ({}), forcing text response",
                    self.loop_config.max_iterations
                );
                break;
            }

            // Check wall-clock timeout for the entire tool loop.
            if loop_start.elapsed() >= self.loop_config.timeout {
                warn!(
                    "Tool loop wall-clock timeout ({:?}) exceeded",
                    self.loop_config.timeout
                );
                break;
            }

            debug!("Agent loop iteration {}", iteration);

            // Snapshot messages and tools for the closure.
            let msgs_snapshot = running_messages.clone();
            let tools_snapshot = openai_tools.clone();

            // Use provider chain with automatic failover for the streaming LLM call.
            // We consume the stream inside the closure and accumulate content + tool calls,
            // emitting ContentChunk events as deltas arrive.
            let event_tx_clone = event_tx.clone();
            let streamed = self
                .provider_chain
                .request_with_failover(None, |provider| {
                    let msgs = msgs_snapshot.clone();
                    let tools = tools_snapshot.clone();
                    let etx = event_tx_clone.clone();
                    async move {
                        consume_stream(provider, msgs, tools, etx).await
                    }
                })
                .await?;

            let content = streamed.content;
            let mut tool_calls = streamed.tool_calls;

            // Enforce per-turn tool call limit.
            if tool_calls.len() > self.loop_config.max_tool_calls_per_turn {
                warn!(
                    "Model requested {} tool calls, capping at {}",
                    tool_calls.len(),
                    self.loop_config.max_tool_calls_per_turn
                );
                tool_calls.truncate(self.loop_config.max_tool_calls_per_turn);
            }

            // Check for tool calls.
            if !tool_calls.is_empty() {
                // Add assistant's message with tool calls to running history.
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

                let assistant_openai = ChatCompletionRequestAssistantMessageArgs::default()
                    .content(&*content)
                    .tool_calls(tc_openai)
                    .build()
                    .map_err(|e| AgentError::Provider(e.to_string()))?;
                running_messages
                    .push(ChatCompletionRequestMessage::Assistant(assistant_openai));

                // Track the assistant tool-call message for session persistence.
                turn_messages.push(Message::assistant_with_tool_calls(
                    &content,
                    tool_calls.clone(),
                ));

                // Execute tool calls concurrently for reduced latency.
                let mut join_set = JoinSet::new();
                let mut immediate_outputs: Vec<(usize, ToolOutput)> = Vec::new();

                for (idx, tc) in tool_calls.iter().enumerate() {
                    let _ = event_tx.send(AgentEvent::ToolCallStart {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                    });

                    // Policy enforcement: reject tools not in the allowed set.
                    if !allowed_tools.contains(&tc.name) {
                        immediate_outputs.push((idx, ToolOutput {
                            tool_call_id: tc.id.clone(),
                            content: format!("Tool not allowed: {}", tc.name),
                            is_error: true,
                        }));
                        continue;
                    }

                    // Parse arguments, returning an error to the model on failure.
                    let args: serde_json::Value = match serde_json::from_str(&tc.arguments) {
                        Ok(v) => v,
                        Err(e) => {
                            immediate_outputs.push((idx, ToolOutput {
                                tool_call_id: tc.id.clone(),
                                content: format!("Invalid JSON arguments: {}", e),
                                is_error: true,
                            }));
                            continue;
                        }
                    };

                    // Spawn concurrent tool execution.
                    let registry = self.tool_registry.clone();
                    let name = tc.name.clone();
                    let id = tc.id.clone();
                    join_set.spawn(async move {
                        let output = registry.execute(&name, &id, args).await;
                        (idx, output)
                    });
                }

                // Collect all results, maintaining original order for deterministic
                // message history.
                let mut indexed_outputs: Vec<(usize, ToolOutput)> = immediate_outputs;
                while let Some(result) = join_set.join_next().await {
                    match result {
                        Ok(pair) => indexed_outputs.push(pair),
                        Err(e) => {
                            warn!("Tool task panicked: {}", e);
                        }
                    }
                }
                indexed_outputs.sort_by_key(|(idx, _)| *idx);

                for (_, output) in indexed_outputs {
                    let _ = event_tx.send(AgentEvent::ToolResult(output.clone()));

                    // Track tool result for session persistence.
                    turn_messages.push(Message::tool_result(
                        &output.tool_call_id,
                        &output.content,
                    ));

                    let tool_msg = ChatCompletionRequestToolMessageArgs::default()
                        .tool_call_id(&output.tool_call_id)
                        .content(&*output.content)
                        .build()
                        .expect("tool message build should not fail");
                    running_messages.push(ChatCompletionRequestMessage::Tool(tool_msg));
                }

                // Continue the loop — the model needs to process tool results.
                continue;
            }

            // No tool calls — this is the final text response.
            // Content chunks were already streamed to event_tx during consume_stream.
            let final_message = Message::assistant(&content);
            let _ = event_tx.send(AgentEvent::Done(final_message.clone()));
            turn_messages.push(final_message);
            return Ok(AgentTurnResult { messages: turn_messages });
        }

        // If we hit max iterations, return whatever we have.
        let fallback = Message::assistant("[Agent reached maximum tool iterations]");
        let _ = event_tx.send(AgentEvent::Done(fallback.clone()));
        turn_messages.push(fallback);
        Ok(AgentTurnResult { messages: turn_messages })
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

/// Accumulated result from consuming a streaming response.
struct StreamedResponse {
    content: String,
    tool_calls: Vec<ToolCall>,
}

/// Open a streaming chat completion, consume all deltas, emit ContentChunk
/// events in real time, and return the accumulated content + tool calls.
async fn consume_stream(
    provider: ResolvedProvider,
    messages: Vec<ChatCompletionRequestMessage>,
    tools: Vec<async_openai::types::ChatCompletionTool>,
    event_tx: mpsc::UnboundedSender<AgentEvent>,
) -> Result<StreamedResponse, RequestError> {
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
    let mut stream = match tokio::time::timeout(
        timeout_duration,
        client.chat().create_stream(request),
    )
    .await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => return classify_provider_error(e),
        Err(_) => {
            return Err(RequestError::Transient(format!(
                "Request timed out after {}s",
                provider.timeout_secs
            )))
        }
    };

    // Accumulate the full content and tool call fragments from streamed deltas.
    let mut content = String::new();
    // Tool calls arrive as indexed chunks: first chunk has id + name, subsequent
    // chunks append to arguments. We accumulate by index.
    let mut tc_ids: HashMap<u32, String> = HashMap::new();
    let mut tc_names: HashMap<u32, String> = HashMap::new();
    let mut tc_args: HashMap<u32, String> = HashMap::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = match chunk_result {
            Ok(c) => c,
            Err(e) => return classify_provider_error(e),
        };

        for choice in &chunk.choices {
            let delta = &choice.delta;

            // Stream content tokens in real time.
            if let Some(text) = &delta.content {
                if !text.is_empty() {
                    let _ = event_tx.send(AgentEvent::ContentChunk(text.clone()));
                    content.push_str(text);
                }
            }

            // Accumulate tool call deltas by index.
            if let Some(tc_chunks) = &delta.tool_calls {
                for tc_chunk in tc_chunks {
                    let idx = tc_chunk.index;

                    if let Some(id) = &tc_chunk.id {
                        tc_ids.insert(idx, id.clone());
                    }

                    if let Some(func) = &tc_chunk.function {
                        if let Some(name) = &func.name {
                            tc_names.insert(idx, name.clone());
                        }
                        if let Some(args) = &func.arguments {
                            tc_args.entry(idx).or_default().push_str(args);
                            let _ = event_tx.send(AgentEvent::ToolCallArgsChunk {
                                id: tc_ids.get(&idx).cloned().unwrap_or_default(),
                                chunk: args.clone(),
                            });
                        }
                    }
                }
            }
        }
    }

    // Assemble accumulated tool calls into final ToolCall objects.
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    let mut indices: Vec<u32> = tc_ids.keys().copied().collect();
    indices.sort();
    for idx in indices {
        tool_calls.push(ToolCall {
            id: tc_ids.remove(&idx).unwrap_or_default(),
            name: tc_names.remove(&idx).unwrap_or_default(),
            arguments: tc_args.remove(&idx).unwrap_or_default(),
        });
    }

    Ok(StreamedResponse {
        content,
        tool_calls,
    })
}

/// Classify an async-openai error for failover decisions.
///
/// Uses structured error matching via `ApiError.code` and `ApiError.type`
/// fields when available, falling back to string matching only for
/// non-API errors (network, deserialization, etc.).
fn classify_provider_error<T>(err: async_openai::error::OpenAIError) -> Result<T, RequestError> {
    use async_openai::error::OpenAIError;

    match &err {
        // Structured API errors — match on code/type fields, not string formatting.
        OpenAIError::ApiError(api_err) => {
            let is_permanent = matches!(
                api_err.code.as_deref(),
                Some("invalid_api_key")
                    | Some("model_not_found")
                    | Some("invalid_request_error")
                    | Some("insufficient_quota")
            ) || matches!(
                api_err.r#type.as_deref(),
                Some("authentication_error")
                    | Some("invalid_request_error")
                    | Some("permission_error")
            );

            if is_permanent {
                Err(RequestError::Permanent(err.to_string()))
            } else {
                // Rate limits, server errors, etc. are transient.
                Err(RequestError::Transient(err.to_string()))
            }
        }
        // Network/HTTP errors from reqwest — always transient.
        OpenAIError::Reqwest(_) => Err(RequestError::Transient(err.to_string())),
        // Stream errors — transient (connection may have dropped).
        OpenAIError::StreamError(_) => Err(RequestError::Transient(err.to_string())),
        // Deserialization, file errors, invalid args — permanent (won't fix on retry).
        _ => Err(RequestError::Permanent(err.to_string())),
    }
}
