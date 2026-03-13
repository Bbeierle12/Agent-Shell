//! Session recording in asciicast v2 format.
//!
//! Records terminal sessions as `.cast` files compatible with asciinema.
//! Ported from ShellVault's `replay::recorder` module.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

/// Errors from recording operations.
#[derive(Debug, thiserror::Error)]
pub enum RecorderError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Recorder not started")]
    NotStarted,
}

/// Asciicast v2 header — first line of a `.cast` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsciicastHeader {
    /// Format version (always 2).
    pub version: u32,
    /// Terminal width in columns.
    pub width: u32,
    /// Terminal height in rows.
    pub height: u32,
    /// Unix timestamp of recording start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
    /// Optional recording title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Optional environment info.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<AsciicastEnv>,
}

/// Environment metadata in the asciicast header.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AsciicastEnv {
    #[serde(rename = "SHELL", skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    #[serde(rename = "TERM", skip_serializing_if = "Option::is_none")]
    pub term: Option<String>,
}

impl AsciicastHeader {
    /// Create a header with the given terminal dimensions.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            version: 2,
            width,
            height,
            timestamp: Some(Utc::now().timestamp()),
            title: None,
            env: Some(AsciicastEnv {
                shell: std::env::var("SHELL").ok(),
                term: std::env::var("TERM").ok(),
            }),
        }
    }
}

/// An event in the asciicast recording.
///
/// Serialized as a JSON array: `[time, type, data]`.
#[derive(Debug, Clone)]
pub struct AsciicastEvent {
    /// Time offset from recording start, in seconds.
    pub time: f64,
    /// Event type: `"o"` for output, `"i"` for input.
    pub event_type: String,
    /// Event data (terminal text).
    pub data: String,
}

/// Records terminal sessions to asciicast v2 format (`.cast` files).
///
/// Usage:
/// ```no_run
/// # use agent_pty::recorder::SessionRecorder;
/// # use std::path::Path;
/// let mut rec = SessionRecorder::new(Path::new("/tmp/demo.cast"), 120, 40);
/// rec.with_title("Demo recording");
/// rec.start().unwrap();
/// rec.record_output(b"$ ls\nfile.txt\n").unwrap();
/// rec.stop().unwrap();
/// ```
pub struct SessionRecorder {
    output_path: PathBuf,
    writer: Option<BufWriter<File>>,
    start_time: chrono::DateTime<Utc>,
    header: AsciicastHeader,
}

impl SessionRecorder {
    /// Create a new recorder that will write to `path`.
    pub fn new(path: &Path, width: u32, height: u32) -> Self {
        Self {
            output_path: path.to_path_buf(),
            writer: None,
            start_time: Utc::now(),
            header: AsciicastHeader::new(width, height),
        }
    }

    /// Set the recording title.
    pub fn with_title(&mut self, title: &str) -> &mut Self {
        self.header.title = Some(title.to_string());
        self
    }

    /// Start recording — creates the file and writes the header line.
    pub fn start(&mut self) -> Result<(), RecorderError> {
        // Ensure parent directory exists.
        if let Some(parent) = self.output_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let file = File::create(&self.output_path)?;
        let mut writer = BufWriter::new(file);

        // Header is written as the first JSON line.
        let header_json = serde_json::to_string(&self.header)?;
        writeln!(writer, "{}", header_json)?;

        self.writer = Some(writer);
        self.start_time = Utc::now();

        Ok(())
    }

    /// Record output data (terminal -> user).
    pub fn record_output(&mut self, data: &[u8]) -> Result<(), RecorderError> {
        self.write_event("o", data)
    }

    /// Record input data (user -> terminal).
    pub fn record_input(&mut self, data: &[u8]) -> Result<(), RecorderError> {
        self.write_event("i", data)
    }

    /// Stop recording and flush the file.
    pub fn stop(&mut self) -> Result<PathBuf, RecorderError> {
        if let Some(ref mut writer) = self.writer {
            writer.flush()?;
        }
        self.writer = None;
        Ok(self.output_path.clone())
    }

    /// Get the output path.
    pub fn output_path(&self) -> &Path {
        &self.output_path
    }

    /// Whether the recorder is currently active.
    pub fn is_recording(&self) -> bool {
        self.writer.is_some()
    }

    // ------ private helpers ------

    fn write_event(&mut self, event_type: &str, data: &[u8]) -> Result<(), RecorderError> {
        let writer = self.writer.as_mut().ok_or(RecorderError::NotStarted)?;

        let elapsed = (Utc::now() - self.start_time).num_milliseconds() as f64 / 1000.0;
        let text = String::from_utf8_lossy(data);

        // Asciicast v2 event format: [time, "o"/"i", data]
        let event = serde_json::json!([elapsed, event_type, text]);
        writeln!(writer, "{}", event)?;

        Ok(())
    }
}

impl Drop for SessionRecorder {
    fn drop(&mut self) {
        if self.writer.is_some() {
            let _ = self.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Helper: create a recorder pointed at a temp directory.
    fn temp_recorder() -> (TempDir, SessionRecorder) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.cast");
        let rec = SessionRecorder::new(&path, 80, 24);
        (dir, rec)
    }

    #[test]
    fn test_recorder_create_write_stop() {
        let (_dir, mut rec) = temp_recorder();

        assert!(!rec.is_recording());
        rec.start().unwrap();
        assert!(rec.is_recording());

        rec.record_output(b"$ whoami\nuser\n").unwrap();
        rec.record_input(b"ls\n").unwrap();

        let path = rec.stop().unwrap();
        assert!(!rec.is_recording());

        // Verify file contents.
        let content = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();

        // First line is the header.
        assert!(lines.len() >= 3, "Expected header + 2 events, got {}", lines.len());

        let header: AsciicastHeader = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(header.version, 2);
        assert_eq!(header.width, 80);
        assert_eq!(header.height, 24);

        // Second line is an output event.
        let ev1: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(ev1[1], "o");
        assert!(ev1[2].as_str().unwrap().contains("whoami"));

        // Third line is an input event.
        let ev2: serde_json::Value = serde_json::from_str(lines[2]).unwrap();
        assert_eq!(ev2[1], "i");
        assert!(ev2[2].as_str().unwrap().contains("ls"));
    }

    #[test]
    fn test_recorder_auto_stop_on_drop() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("drop.cast");

        {
            let mut rec = SessionRecorder::new(&path, 120, 40);
            rec.start().unwrap();
            rec.record_output(b"hello from drop test\n").unwrap();
            // rec is dropped here without calling stop()
        }

        // File should still have been flushed and closed.
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("hello from drop test"));
    }

    #[test]
    fn test_recorder_with_title() {
        let (_dir, mut rec) = temp_recorder();
        rec.with_title("My Session");
        rec.start().unwrap();
        let path = rec.stop().unwrap();

        let content = fs::read_to_string(path).unwrap();
        let header: AsciicastHeader = serde_json::from_str(content.lines().next().unwrap()).unwrap();
        assert_eq!(header.title.as_deref(), Some("My Session"));
    }

    #[test]
    fn test_recorder_error_before_start() {
        let (_dir, mut rec) = temp_recorder();
        let result = rec.record_output(b"should fail");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not started"));
    }
}
