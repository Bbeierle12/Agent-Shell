//! PTY session management.
//!
//! Wraps `portable-pty` to provide a managed PTY session with async I/O.
//! Ported from netsec-pty.

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;

use crate::shell::ShellInfo;

/// Errors that can occur during PTY operations.
#[derive(Debug, thiserror::Error)]
pub enum PtyError {
    #[error("Failed to create PTY: {0}")]
    Creation(String),

    #[error("Failed to spawn shell: {0}")]
    Spawn(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("PTY not initialized")]
    NotInitialized,
}

/// A PTY session wrapping a shell process.
///
/// The `PtyPair` is split during construction: the slave is consumed to
/// spawn the shell, and the master is retained (behind a `std::sync::Mutex`)
/// for resize operations.  Reader and writer are extracted from the master
/// and wrapped in `tokio::sync::Mutex` for async access.
pub struct PtySession {
    master: StdMutex<Box<dyn MasterPty + Send>>,
    reader: Arc<Mutex<Box<dyn Read + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    shell: ShellInfo,
    size: StdMutex<PtySize>,
}

// Safety: all interior fields are either Send+Sync (StdMutex, Arc<Mutex>)
// or Send behind a StdMutex which provides Sync.
unsafe impl Sync for PtySession {}

impl PtySession {
    /// Create a new PTY session with the given shell and dimensions.
    pub fn new(shell: &ShellInfo, cols: u16, rows: u16) -> Result<Self, PtyError> {
        let pty_system = native_pty_system();

        let size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = pty_system
            .openpty(size)
            .map_err(|e| PtyError::Creation(e.to_string()))?;

        // Build the command.
        let mut cmd = CommandBuilder::new(&shell.path);

        #[cfg(windows)]
        {
            cmd.env("TERM", "xterm-256color");
        }

        #[cfg(unix)]
        {
            cmd.env("TERM", "xterm-256color");
            cmd.env("COLORTERM", "truecolor");
        }

        // Spawn the shell (consumes the slave).
        let _child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| PtyError::Spawn(e.to_string()))?;

        // Extract reader and writer from the master.
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| PtyError::Creation(e.to_string()))?;

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| PtyError::Creation(e.to_string()))?;

        tracing::debug!(
            shell = %shell.id,
            cols = cols,
            rows = rows,
            "PTY session created"
        );

        Ok(Self {
            master: StdMutex::new(pair.master),
            reader: Arc::new(Mutex::new(reader)),
            writer: Arc::new(Mutex::new(writer)),
            shell: shell.clone(),
            size: StdMutex::new(size),
        })
    }

    /// Get the shell info for this session.
    pub fn shell(&self) -> &ShellInfo {
        &self.shell
    }

    /// Get the current terminal size (cols, rows).
    pub fn size(&self) -> (u16, u16) {
        let s = self.size.lock().unwrap();
        (s.cols, s.rows)
    }

    /// Resize the terminal.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), PtyError> {
        let new_size = PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        let master = self.master.lock().unwrap();
        master
            .resize(new_size)
            .map_err(|e| PtyError::Creation(e.to_string()))?;

        *self.size.lock().unwrap() = new_size;

        tracing::debug!(cols = cols, rows = rows, "PTY resized");
        Ok(())
    }

    /// Write data to the PTY (send input to the shell).
    pub async fn write(&self, data: &[u8]) -> Result<(), PtyError> {
        let mut writer = self.writer.lock().await;
        writer.write_all(data)?;
        writer.flush()?;
        Ok(())
    }

    /// Read available data from the PTY (get output from the shell).
    ///
    /// This is a blocking read — call from a dedicated thread/task via
    /// `tokio::task::spawn_blocking`.
    pub fn read_blocking(&self, buf: &mut [u8]) -> Result<usize, PtyError> {
        let mut reader = self.reader.blocking_lock();
        match reader.read(buf) {
            Ok(n) => Ok(n),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(0),
            Err(e) => Err(PtyError::Io(e)),
        }
    }

    /// Get a clone of the reader for use in background tasks.
    pub fn reader(&self) -> Arc<Mutex<Box<dyn Read + Send>>> {
        Arc::clone(&self.reader)
    }
}

impl std::fmt::Debug for PtySession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (cols, rows) = self.size();
        f.debug_struct("PtySession")
            .field("shell", &self.shell.id)
            .field("size", &format!("{}x{}", cols, rows))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pty_error_display() {
        let err = PtyError::Creation("test error".to_string());
        assert_eq!(err.to_string(), "Failed to create PTY: test error");

        let err = PtyError::Spawn("spawn error".to_string());
        assert_eq!(err.to_string(), "Failed to spawn shell: spawn error");

        let err = PtyError::NotInitialized;
        assert_eq!(err.to_string(), "PTY not initialized");
    }

    #[test]
    fn test_pty_session_creation() {
        // This test requires a real shell — only run in environments where one exists.
        let shell = crate::shell::default_shell();
        if shell.is_none() {
            return;
        }
        let shell = shell.unwrap();

        let session = PtySession::new(&shell, 80, 24);
        assert!(
            session.is_ok(),
            "Should create PTY session: {:?}",
            session.err()
        );

        let session = session.unwrap();
        assert_eq!(session.size(), (80, 24));
        assert_eq!(session.shell().id, shell.id);
    }

    #[test]
    fn test_pty_session_debug() {
        let shell = crate::shell::default_shell();
        if shell.is_none() {
            return;
        }
        let shell = shell.unwrap();

        let session = PtySession::new(&shell, 120, 40).unwrap();
        let debug = format!("{:?}", session);
        assert!(debug.contains("PtySession"));
        assert!(debug.contains("120x40"));
    }

    #[test]
    fn test_pty_session_resize() {
        let shell = crate::shell::default_shell();
        if shell.is_none() {
            return;
        }
        let shell = shell.unwrap();

        let session = PtySession::new(&shell, 80, 24).unwrap();
        assert_eq!(session.size(), (80, 24));

        session.resize(120, 40).unwrap();
        assert_eq!(session.size(), (120, 40));
    }
}
