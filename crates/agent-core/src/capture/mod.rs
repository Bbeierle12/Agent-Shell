//! Shell hook capture system.
//!
//! Provides the hook-based capture backend that receives messages from shell
//! integrations (bash/zsh/fish) and converts them into [`CaptureEvent`]s.
//! Ported from ShellVault's capture subsystem.
//!
//! # Modules
//!
//! - [`types`] -- Hook message types and protocol versioning.
//! - [`hook`] -- The [`HookBackend`] that processes hook messages into events.

pub mod hook;
pub mod types;

pub use hook::{CaptureEvent, HookBackend};
pub use types::{
    HookEventType, HookMessage, HookMessageV1, LegacyHookEventType, LegacyHookMessage,
    HOOK_PROTOCOL_VERSION,
};
