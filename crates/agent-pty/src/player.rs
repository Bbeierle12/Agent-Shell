//! Terminal session playback.
//!
//! Reads asciicast v2 `.cast` files and exposes them as an iterator of
//! timed events.  Supports seek, reset, speed control, and transcript
//! extraction.  Ported from ShellVault's `replay::player` module.

use crate::recorder::AsciicastHeader;
use serde_json::Value;
use std::path::Path;
use std::time::Duration;

/// Errors from playback operations.
#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Invalid recording: {0}")]
    InvalidRecording(String),

    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
}

/// The type of a playback event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventType {
    /// Terminal output (shell -> screen).
    Output,
    /// Terminal input (keyboard -> shell).
    Input,
}

/// A single event during playback.
#[derive(Debug, Clone)]
pub struct PlaybackEvent {
    /// Time offset from recording start.
    pub time: Duration,
    /// Whether this is output or input.
    pub event_type: EventType,
    /// The text data.
    pub data: String,
}

/// Plays back asciicast v2 recordings.
#[derive(Debug)]
pub struct ReplayPlayer {
    header: AsciicastHeader,
    events: Vec<PlaybackEvent>,
    current_index: usize,
    speed: f64,
}

impl ReplayPlayer {
    /// Load a recording from a `.cast` file on disk.
    pub fn load(path: &Path) -> Result<Self, PlayerError> {
        let content = std::fs::read_to_string(path)?;
        Self::from_str(&content)
    }

    /// Parse a recording from an in-memory string.
    pub fn from_str(content: &str) -> Result<Self, PlayerError> {
        let mut lines = content.lines();

        // First line is the header.
        let header_line = lines
            .next()
            .ok_or_else(|| PlayerError::InvalidRecording("Empty file".into()))?;
        let header: AsciicastHeader = serde_json::from_str(header_line)
            .map_err(|e| PlayerError::InvalidRecording(format!("Invalid header: {}", e)))?;

        // Remaining lines are events.
        let mut events = Vec::new();
        for line in lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let value: Value = serde_json::from_str(line)
                .map_err(|e| PlayerError::InvalidRecording(format!("Invalid event: {}", e)))?;

            if let Value::Array(ref arr) = value {
                if arr.len() >= 3 {
                    let time = arr[0].as_f64().unwrap_or(0.0);
                    let event_type = match arr[1].as_str().unwrap_or("o") {
                        "i" => EventType::Input,
                        _ => EventType::Output,
                    };
                    let data = arr[2].as_str().unwrap_or("").to_string();

                    events.push(PlaybackEvent {
                        time: Duration::from_secs_f64(time),
                        event_type,
                        data,
                    });
                }
            }
        }

        Ok(Self {
            header,
            events,
            current_index: 0,
            speed: 1.0,
        })
    }

    /// Get the asciicast header.
    pub fn header(&self) -> &AsciicastHeader {
        &self.header
    }

    /// Terminal dimensions `(width, height)`.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.header.width, self.header.height)
    }

    /// Total duration of the recording (time of last event).
    pub fn duration(&self) -> Duration {
        self.events
            .last()
            .map(|e| e.time)
            .unwrap_or(Duration::ZERO)
    }

    /// Total number of events.
    pub fn event_count(&self) -> usize {
        self.events.len()
    }

    /// Set playback speed (clamped to 0.1..=10.0).
    pub fn set_speed(&mut self, speed: f64) {
        self.speed = speed.clamp(0.1, 10.0);
    }

    /// Current playback speed.
    pub fn speed(&self) -> f64 {
        self.speed
    }

    /// Reset playback to the beginning.
    pub fn reset(&mut self) {
        self.current_index = 0;
    }

    /// Seek to the first event at or after `time`.
    pub fn seek(&mut self, time: Duration) {
        self.current_index = self
            .events
            .iter()
            .position(|e| e.time >= time)
            .unwrap_or(self.events.len());
    }

    /// Advance and return the next event, or `None` if playback is done.
    pub fn next_event(&mut self) -> Option<&PlaybackEvent> {
        if self.current_index < self.events.len() {
            let event = &self.events[self.current_index];
            self.current_index += 1;
            Some(event)
        } else {
            None
        }
    }

    /// Peek at the next event without advancing.
    pub fn peek_event(&self) -> Option<&PlaybackEvent> {
        self.events.get(self.current_index)
    }

    /// Current playback position (event index).
    pub fn position(&self) -> usize {
        self.current_index
    }

    /// Whether playback has reached the end.
    pub fn is_complete(&self) -> bool {
        self.current_index >= self.events.len()
    }

    /// Borrow all events.
    pub fn events(&self) -> &[PlaybackEvent] {
        &self.events
    }

    /// Concatenate all output events into a single transcript string.
    pub fn transcript(&self) -> String {
        self.events
            .iter()
            .filter(|e| e.event_type == EventType::Output)
            .map(|e| e.data.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal valid asciicast v2 recording for tests.
    fn sample_cast() -> String {
        [
            r#"{"version":2,"width":80,"height":24}"#,
            r#"[0.5,"o","$ ls\r\n"]"#,
            r#"[1.0,"o","file.txt\r\n"]"#,
            r#"[1.2,"i","pwd\r\n"]"#,
            r#"[2.0,"o","/home/user\r\n"]"#,
            r#"[3.5,"o","$ "]"#,
        ]
        .join("\n")
    }

    #[test]
    fn test_player_load_from_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.cast");
        std::fs::write(&path, sample_cast()).unwrap();

        let player = ReplayPlayer::load(&path).unwrap();
        assert_eq!(player.event_count(), 5);
        assert_eq!(player.dimensions(), (80, 24));
    }

    #[test]
    fn test_player_from_str_event_count() {
        let player = ReplayPlayer::from_str(&sample_cast()).unwrap();
        assert_eq!(player.event_count(), 5);
    }

    #[test]
    fn test_player_seek() {
        let mut player = ReplayPlayer::from_str(&sample_cast()).unwrap();

        // Seek to 1.0s — should land on the second event.
        player.seek(Duration::from_secs_f64(1.0));
        let ev = player.next_event().unwrap();
        assert_eq!(ev.time, Duration::from_secs_f64(1.0));
        assert!(ev.data.contains("file.txt"));

        // Seek past the end.
        player.seek(Duration::from_secs(100));
        assert!(player.is_complete());
    }

    #[test]
    fn test_player_transcript() {
        let player = ReplayPlayer::from_str(&sample_cast()).unwrap();
        let transcript = player.transcript();
        // Should contain output events only (not the "i" event).
        assert!(transcript.contains("$ ls"));
        assert!(transcript.contains("file.txt"));
        assert!(transcript.contains("/home/user"));
        assert!(!transcript.contains("pwd")); // input event excluded
    }

    #[test]
    fn test_player_duration() {
        let player = ReplayPlayer::from_str(&sample_cast()).unwrap();
        let dur = player.duration();
        // Last event is at 3.5 seconds.
        assert!((dur.as_secs_f64() - 3.5).abs() < 0.001);
    }

    #[test]
    fn test_player_reset() {
        let mut player = ReplayPlayer::from_str(&sample_cast()).unwrap();

        // Consume some events.
        player.next_event();
        player.next_event();
        assert_eq!(player.position(), 2);

        player.reset();
        assert_eq!(player.position(), 0);
        assert!(!player.is_complete());
    }

    #[test]
    fn test_player_invalid_file() {
        let result = ReplayPlayer::from_str("");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Empty file"));
    }
}
