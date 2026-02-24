//! Cross-platform PTY abstraction for terminal emulation in agent-shell.
//!
//! Provides shell detection, PTY session management, and capture events.
//! Ported from netsec-pty with ShellVault capture integration.

pub mod capture;
pub mod session;
pub mod shell;

pub use capture::CaptureEvent;
pub use session::{PtyError, PtySession};
pub use shell::{detect_available_shells, default_shell, ShellInfo};
