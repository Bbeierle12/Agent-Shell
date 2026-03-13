//! Structured ReAct tool loop — ported from GlyphX's agent pattern
//! (New-Idea/glyphx/app/services/tools.py + worker.py).
//!
//! This module provides a provider-agnostic representation of the
//! Think -> Act -> Observe -> Repeat cycle that underpins tool-calling
//! agents. It is intentionally decoupled from any specific LLM client;
//! callers supply a closure that performs the actual LLM call.
//!
//! The existing `AgentLoop` in agent_loop.rs handles the full
//! async-openai integration. This module captures the *structural*
//! pattern so other subsystems (skills, plugins, tests) can reason
//! about the loop without pulling in the full LLM stack.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::AgentError;

// ---------------------------------------------------------------------------
// ToolLoopConfig
// ---------------------------------------------------------------------------

/// Configuration knobs for a tool-calling loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolLoopConfig {
    /// Maximum Think-Act-Observe iterations before the loop terminates.
    pub max_iterations: usize,
    /// Maximum distinct tool calls allowed within a single iteration.
    pub max_tool_calls_per_turn: usize,
    /// Overall wall-clock timeout for the entire loop.
    pub timeout: Duration,
}

impl Default for ToolLoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: 20,
            max_tool_calls_per_turn: 10,
            timeout: Duration::from_secs(300), // 5 minutes
        }
    }
}

impl ToolLoopConfig {
    /// Validate the config, returning an error if any invariant is violated.
    pub fn validate(&self) -> Result<(), AgentError> {
        if self.max_iterations == 0 {
            return Err(AgentError::Config(
                "max_iterations must be > 0".into(),
            ));
        }
        if self.max_tool_calls_per_turn == 0 {
            return Err(AgentError::Config(
                "max_tool_calls_per_turn must be > 0".into(),
            ));
        }
        if self.timeout.is_zero() {
            return Err(AgentError::Config("timeout must be > 0".into()));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ToolLoopStep
// ---------------------------------------------------------------------------

/// One iteration of the ReAct loop, capturing the model's reasoning,
/// the action it chose, and the observation it received.
///
/// Modelled after GlyphX's `ToolsBridge.execute_tool` pattern where every
/// tool invocation is wrapped in (name, arguments) -> result, but elevated
/// to include the reasoning trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolLoopStep {
    /// The model's reasoning / chain-of-thought before acting.
    pub thought: String,
    /// The action taken (tool name + arguments), if any.
    pub action: Option<ToolAction>,
    /// The observation returned by the tool (or the environment).
    pub observation: String,
    /// Whether this step produced the final answer (loop should stop).
    pub is_final: bool,
}

/// Represents a single tool invocation within a step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolAction {
    /// Tool name (matches `ToolSchema.name` in the registry).
    pub tool_name: String,
    /// JSON-encoded arguments.
    pub arguments: serde_json::Value,
    /// Unique call ID for correlating results.
    pub call_id: String,
}

// ---------------------------------------------------------------------------
// ToolLoopOutcome
// ---------------------------------------------------------------------------

/// The result of running a full tool loop.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolLoopOutcome {
    /// All steps executed during the loop.
    pub steps: Vec<ToolLoopStep>,
    /// The final textual answer produced by the model.
    pub final_answer: String,
    /// Whether the loop terminated normally (model said "done") or was
    /// cut short by hitting `max_iterations` / timeout.
    pub completed: bool,
    /// Number of iterations executed.
    pub iterations: usize,
}

// ---------------------------------------------------------------------------
// run_tool_loop (sync, closure-based)
// ---------------------------------------------------------------------------

/// Execute a structured ReAct tool loop.
///
/// This is a **synchronous** driver intended for use in tests, CLI tools,
/// and non-async contexts. For the full async agent loop that talks to
/// real LLM APIs, see `AgentLoop::run` in `agent_loop.rs`.
///
/// # Arguments
///
/// * `prompt` — the user's request
/// * `tools` — list of available tool names (for display / validation)
/// * `config` — loop configuration
/// * `step_fn` — closure called each iteration with `(iteration, history)`.
///   It must return the next `ToolLoopStep`. Return a step with
///   `is_final = true` to end the loop.
///
/// # Errors
///
/// Returns `AgentError::Config` if the config is invalid, or propagates
/// errors from `step_fn`.
pub fn run_tool_loop<F>(
    prompt: &str,
    _tools: &[String],
    config: &ToolLoopConfig,
    mut step_fn: F,
) -> Result<ToolLoopOutcome, AgentError>
where
    F: FnMut(usize, &[ToolLoopStep]) -> Result<ToolLoopStep, AgentError>,
{
    config.validate()?;

    let start = std::time::Instant::now();
    let mut steps: Vec<ToolLoopStep> = Vec::new();
    let mut iterations = 0;

    for i in 0..config.max_iterations {
        if start.elapsed() >= config.timeout {
            break;
        }

        iterations = i + 1;
        let step = step_fn(i, &steps)?;
        let is_final = step.is_final;
        steps.push(step);

        if is_final {
            let final_answer = steps
                .last()
                .map(|s| s.observation.clone())
                .unwrap_or_default();
            return Ok(ToolLoopOutcome {
                steps,
                final_answer,
                completed: true,
                iterations,
            });
        }
    }

    // Loop ended without a final step — either max_iterations or timeout.
    let final_answer = steps
        .last()
        .map(|s| s.observation.clone())
        .unwrap_or_else(|| format!("Loop ended without answer for: {}", prompt));

    Ok(ToolLoopOutcome {
        steps,
        final_answer,
        completed: false,
        iterations,
    })
}

// ===========================================================================
// Tests
// ===========================================================================
#[cfg(test)]
mod tests {
    use super::*;

    // --- ToolLoopConfig: defaults -----------------------------------------
    #[test]
    fn test_config_defaults() {
        let cfg = ToolLoopConfig::default();
        assert_eq!(cfg.max_iterations, 20);
        assert_eq!(cfg.max_tool_calls_per_turn, 10);
        assert_eq!(cfg.timeout, Duration::from_secs(300));
        assert!(cfg.validate().is_ok());
    }

    // --- ToolLoopConfig: validation (max_iterations > 0) ------------------
    #[test]
    fn test_config_validation_zero_iterations() {
        let cfg = ToolLoopConfig {
            max_iterations: 0,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("max_iterations"));
    }

    #[test]
    fn test_config_validation_zero_tool_calls() {
        let cfg = ToolLoopConfig {
            max_tool_calls_per_turn: 0,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("max_tool_calls_per_turn"));
    }

    #[test]
    fn test_config_validation_zero_timeout() {
        let cfg = ToolLoopConfig {
            timeout: Duration::ZERO,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.to_string().contains("timeout"));
    }

    // --- ToolLoopStep: serialization roundtrip ----------------------------
    #[test]
    fn test_step_serialization() {
        let step = ToolLoopStep {
            thought: "I need to list files first.".into(),
            action: Some(ToolAction {
                tool_name: "list_files".into(),
                arguments: serde_json::json!({"path": "/tmp"}),
                call_id: "call-001".into(),
            }),
            observation: "file1.txt\nfile2.txt".into(),
            is_final: false,
        };

        let json = serde_json::to_string(&step).unwrap();
        let parsed: ToolLoopStep = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.thought, "I need to list files first.");
        assert!(parsed.action.is_some());
        let action = parsed.action.unwrap();
        assert_eq!(action.tool_name, "list_files");
        assert_eq!(action.call_id, "call-001");
        assert!(!parsed.is_final);
    }

    // --- ToolLoopStep: final step with no action --------------------------
    #[test]
    fn test_step_final_no_action() {
        let step = ToolLoopStep {
            thought: "I have all the information.".into(),
            action: None,
            observation: "The answer is 42.".into(),
            is_final: true,
        };

        let json = serde_json::to_string(&step).unwrap();
        let parsed: ToolLoopStep = serde_json::from_str(&json).unwrap();

        assert!(parsed.is_final);
        assert!(parsed.action.is_none());
        assert_eq!(parsed.observation, "The answer is 42.");
    }

    // --- ToolLoopConfig: serialization roundtrip --------------------------
    #[test]
    fn test_config_serialization() {
        let cfg = ToolLoopConfig {
            max_iterations: 5,
            max_tool_calls_per_turn: 3,
            timeout: Duration::from_secs(60),
        };

        let json = serde_json::to_string(&cfg).unwrap();
        let parsed: ToolLoopConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.max_iterations, 5);
        assert_eq!(parsed.max_tool_calls_per_turn, 3);
        assert_eq!(parsed.timeout, Duration::from_secs(60));
    }

    // --- ToolLoopOutcome: serialization -----------------------------------
    #[test]
    fn test_outcome_serialization() {
        let outcome = ToolLoopOutcome {
            steps: vec![],
            final_answer: "done".into(),
            completed: true,
            iterations: 1,
        };

        let json = serde_json::to_string(&outcome).unwrap();
        let parsed: ToolLoopOutcome = serde_json::from_str(&json).unwrap();
        assert!(parsed.completed);
        assert_eq!(parsed.final_answer, "done");
    }

    // --- run_tool_loop: simple two-step loop ------------------------------
    #[test]
    fn test_run_tool_loop_two_steps() {
        let config = ToolLoopConfig {
            max_iterations: 10,
            max_tool_calls_per_turn: 5,
            timeout: Duration::from_secs(30),
        };

        let tools = vec!["read_file".into(), "write_file".into()];

        let outcome = run_tool_loop("Read /tmp/test.txt", &tools, &config, |i, _history| {
            if i == 0 {
                Ok(ToolLoopStep {
                    thought: "I should read the file.".into(),
                    action: Some(ToolAction {
                        tool_name: "read_file".into(),
                        arguments: serde_json::json!({"path": "/tmp/test.txt"}),
                        call_id: "call-1".into(),
                    }),
                    observation: "Hello, world!".into(),
                    is_final: false,
                })
            } else {
                Ok(ToolLoopStep {
                    thought: "I have the file contents.".into(),
                    action: None,
                    observation: "The file contains: Hello, world!".into(),
                    is_final: true,
                })
            }
        })
        .unwrap();

        assert!(outcome.completed);
        assert_eq!(outcome.iterations, 2);
        assert_eq!(outcome.steps.len(), 2);
        assert_eq!(
            outcome.final_answer,
            "The file contains: Hello, world!"
        );
    }

    // --- run_tool_loop: hits max_iterations without finishing --------------
    #[test]
    fn test_run_tool_loop_max_iterations() {
        let config = ToolLoopConfig {
            max_iterations: 3,
            max_tool_calls_per_turn: 5,
            timeout: Duration::from_secs(30),
        };

        let outcome = run_tool_loop("loop forever", &[], &config, |i, _| {
            Ok(ToolLoopStep {
                thought: format!("Iteration {}", i),
                action: None,
                observation: format!("Still going at {}", i),
                is_final: false,
            })
        })
        .unwrap();

        assert!(!outcome.completed);
        assert_eq!(outcome.iterations, 3);
        assert_eq!(outcome.steps.len(), 3);
    }

    // --- run_tool_loop: invalid config is rejected ------------------------
    #[test]
    fn test_run_tool_loop_invalid_config() {
        let config = ToolLoopConfig {
            max_iterations: 0,
            ..Default::default()
        };

        let result = run_tool_loop("test", &[], &config, |_, _| {
            unreachable!("should not be called");
        });
        assert!(result.is_err());
    }

    // --- run_tool_loop: immediate final answer ----------------------------
    #[test]
    fn test_run_tool_loop_immediate_final() {
        let config = ToolLoopConfig::default();

        let outcome = run_tool_loop("what is 2+2?", &[], &config, |_i, _| {
            Ok(ToolLoopStep {
                thought: "Simple math.".into(),
                action: None,
                observation: "4".into(),
                is_final: true,
            })
        })
        .unwrap();

        assert!(outcome.completed);
        assert_eq!(outcome.iterations, 1);
        assert_eq!(outcome.final_answer, "4");
    }

    // --- run_tool_loop: step_fn error propagates --------------------------
    #[test]
    fn test_run_tool_loop_step_error() {
        let config = ToolLoopConfig::default();

        let result = run_tool_loop("fail", &[], &config, |_, _| {
            Err(AgentError::ToolExecution {
                tool_name: "bad_tool".into(),
                message: "boom".into(),
            })
        });

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("boom"));
    }

    // --- ToolAction: serialization ----------------------------------------
    #[test]
    fn test_tool_action_serialization() {
        let action = ToolAction {
            tool_name: "run_shell".into(),
            arguments: serde_json::json!({"command": "ls -la"}),
            call_id: "tc-42".into(),
        };

        let json = serde_json::to_string(&action).unwrap();
        let parsed: ToolAction = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tool_name, "run_shell");
        assert_eq!(parsed.call_id, "tc-42");
    }
}
