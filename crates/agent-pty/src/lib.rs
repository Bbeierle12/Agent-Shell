//! Cross-platform PTY abstraction for terminal emulation in agent-shell.
//!
//! Provides shell detection, PTY session management, capture events,
//! and terminal recording / playback in asciicast v2 format.
//! Ported from netsec-pty with ShellVault capture and replay integration.

pub mod capture;
pub mod export;
pub mod player;
pub mod recorder;
pub mod session;
pub mod shell;

pub use capture::CaptureEvent;
pub use export::RecordingExporter;
pub use player::{EventType, PlaybackEvent, PlayerError, ReplayPlayer};
pub use recorder::{AsciicastEvent, AsciicastHeader, RecorderError, SessionRecorder};
pub use session::{PtyError, PtySession};
pub use shell::{default_shell, detect_available_shells, ShellInfo};
