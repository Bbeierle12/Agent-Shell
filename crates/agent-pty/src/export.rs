//! Export recordings to various formats.
//!
//! Converts asciicast v2 recordings to plain-text transcripts, markdown
//! documents, or secret-scrubbed asciicast files.
//! Ported from ShellVault's `replay::export` module.

use crate::player::{EventType, PlayerError, ReplayPlayer};
use std::path::Path;

/// Exports recordings to text, markdown, or scrubbed asciicast.
pub struct RecordingExporter;

impl RecordingExporter {
    /// Extract a plain-text transcript from a `.cast` file.
    ///
    /// Only output events are included; input events are omitted.
    pub fn to_text(recording_path: &Path) -> Result<String, PlayerError> {
        let player = ReplayPlayer::load(recording_path)?;
        Ok(player.transcript())
    }

    /// Export a recording as a markdown document.
    ///
    /// Includes a title, duration, dimensions, and a fenced code block of
    /// the output transcript with timestamp markers for gaps > 2 s.
    pub fn to_markdown(recording_path: &Path, title: &str) -> Result<String, PlayerError> {
        let player = ReplayPlayer::load(recording_path)?;

        let mut out = String::new();
        out.push_str(&format!("# {}\n\n", title));
        out.push_str(&format!(
            "**Duration:** {:.1}s  \n",
            player.duration().as_secs_f64()
        ));
        out.push_str(&format!(
            "**Dimensions:** {}x{}\n\n",
            player.header().width,
            player.header().height
        ));
        out.push_str("## Transcript\n\n```\n");

        let mut current_text = String::new();
        let mut last_time = std::time::Duration::ZERO;

        for event in player.events() {
            if event.event_type == EventType::Output {
                // Insert a timestamp marker for gaps longer than 2 seconds.
                if event.time.saturating_sub(last_time) > std::time::Duration::from_secs(2) {
                    if !current_text.is_empty() {
                        out.push_str(&current_text);
                        current_text.clear();
                    }
                    out.push_str(&format!(
                        "\n--- [{:.1}s] ---\n",
                        event.time.as_secs_f64()
                    ));
                }
                current_text.push_str(&event.data);
                last_time = event.time;
            }
        }

        if !current_text.is_empty() {
            out.push_str(&current_text);
        }
        out.push_str("\n```\n");

        Ok(out)
    }

    /// Export a recording as asciicast with patterns scrubbed from the data.
    ///
    /// Every occurrence of each pattern in event data is replaced with
    /// `[REDACTED]`.  This is useful for removing API keys, tokens, or
    /// passwords before sharing a recording.
    pub fn to_asciicast_scrubbed(
        recording_path: &Path,
        patterns: &[&str],
    ) -> Result<String, PlayerError> {
        let player = ReplayPlayer::load(recording_path)?;

        let mut out = String::new();

        // Re-serialize the header.
        let header_json = serde_json::to_string(player.header())
            .map_err(|e| PlayerError::InvalidRecording(e.to_string()))?;
        out.push_str(&header_json);
        out.push('\n');

        // Re-serialize each event, scrubbing data.
        for event in player.events() {
            let mut data = event.data.clone();
            for pat in patterns {
                if !pat.is_empty() {
                    data = data.replace(pat, "[REDACTED]");
                }
            }

            let event_type_str = match event.event_type {
                EventType::Output => "o",
                EventType::Input => "i",
            };

            let json =
                serde_json::json!([event.time.as_secs_f64(), event_type_str, data]);
            out.push_str(&json.to_string());
            out.push('\n');
        }

        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Write a sample cast file and return its path and temp dir.
    fn write_sample_cast() -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sample.cast");
        let content = [
            r#"{"version":2,"width":80,"height":24,"title":"Test Session"}"#,
            r#"[0.1,"o","$ echo $SECRET\r\n"]"#,
            r#"[0.5,"o","my-api-key-12345\r\n"]"#,
            r#"[1.0,"i","exit\r\n"]"#,
            r#"[3.5,"o","$ done\r\n"]"#,
        ]
        .join("\n");
        fs::write(&path, content).unwrap();
        (dir, path)
    }

    #[test]
    fn test_to_text_clean_transcript() {
        let (_dir, path) = write_sample_cast();
        let text = RecordingExporter::to_text(&path).unwrap();
        assert!(text.contains("echo $SECRET"));
        assert!(text.contains("my-api-key-12345"));
        assert!(text.contains("$ done"));
        // Input events should not appear.
        assert!(!text.contains("exit"));
    }

    #[test]
    fn test_to_markdown_formatting() {
        let (_dir, path) = write_sample_cast();
        let md = RecordingExporter::to_markdown(&path, "Demo Session").unwrap();

        assert!(md.starts_with("# Demo Session\n"));
        assert!(md.contains("**Duration:**"));
        assert!(md.contains("**Dimensions:** 80x24"));
        assert!(md.contains("## Transcript"));
        assert!(md.contains("```"));
        // Should have a timestamp marker for the gap between 0.5s and 3.5s.
        assert!(md.contains("--- [3.5s] ---"));
    }

    #[test]
    fn test_scrubbed_export_removes_patterns() {
        let (_dir, path) = write_sample_cast();
        let scrubbed = RecordingExporter::to_asciicast_scrubbed(
            &path,
            &["my-api-key-12345", "$SECRET"],
        )
        .unwrap();

        assert!(!scrubbed.contains("my-api-key-12345"));
        assert!(!scrubbed.contains("$SECRET"));
        assert!(scrubbed.contains("[REDACTED]"));

        // The scrubbed output should still be valid asciicast.
        let player = ReplayPlayer::from_str(&scrubbed).unwrap();
        assert_eq!(player.event_count(), 4);
    }
}
