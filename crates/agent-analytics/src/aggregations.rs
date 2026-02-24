//! Session analytics and time/frequency aggregations.
//!
//! Processes agent-shell sessions to compute daily activity summaries,
//! tool usage frequency, conversation metrics, and deep work detection.

use agent_core::session::Session;
use agent_core::types::Role;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Daily activity summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailySummary {
    pub date: NaiveDate,
    /// Total active time in seconds (sum of session durations).
    pub total_active_time_secs: u64,
    /// Number of sessions on this day.
    pub session_count: u32,
    /// Total messages across all sessions.
    pub message_count: u32,
    /// Total user messages.
    pub user_message_count: u32,
    /// Total assistant messages.
    pub assistant_message_count: u32,
    /// Total tool calls.
    pub tool_call_count: u32,
    /// Total tool errors (tool results flagged as errors).
    pub tool_error_count: u32,
    /// Top tools used, sorted by frequency.
    pub top_tools: Vec<(String, u32)>,
    /// Tags seen across sessions.
    pub tags: Vec<String>,
}

impl DailySummary {
    pub fn new(date: NaiveDate) -> Self {
        Self {
            date,
            total_active_time_secs: 0,
            session_count: 0,
            message_count: 0,
            user_message_count: 0,
            assistant_message_count: 0,
            tool_call_count: 0,
            tool_error_count: 0,
            top_tools: Vec::new(),
            tags: Vec::new(),
        }
    }

    /// Error rate as a fraction (0.0..1.0).
    pub fn tool_error_rate(&self) -> f64 {
        if self.tool_call_count == 0 {
            0.0
        } else {
            self.tool_error_count as f64 / self.tool_call_count as f64
        }
    }
}

/// Statistics computed for a single session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStats {
    pub session_id: String,
    pub session_name: String,
    pub date: NaiveDate,
    /// Duration in seconds from first to last message.
    pub duration_secs: u64,
    pub message_count: u32,
    pub user_message_count: u32,
    pub assistant_message_count: u32,
    pub tool_call_count: u32,
    pub tool_error_count: u32,
    /// Distinct tools used.
    pub tools_used: Vec<String>,
}

/// Analytics engine for computing metrics across sessions.
pub struct Analytics {
    /// Tool usage frequency counter.
    tool_counts: HashMap<String, u32>,
    /// Daily summaries keyed by date.
    daily_summaries: HashMap<NaiveDate, DailySummary>,
    /// Per-day tool counts (used to finalize top_tools on each summary).
    daily_tool_counts: HashMap<NaiveDate, HashMap<String, u32>>,
    /// Deep work threshold in minutes.
    deep_work_threshold_mins: u32,
    /// Processed session stats.
    session_stats: Vec<SessionStats>,
}

impl Analytics {
    pub fn new(deep_work_threshold_mins: u32) -> Self {
        Self {
            tool_counts: HashMap::new(),
            daily_summaries: HashMap::new(),
            daily_tool_counts: HashMap::new(),
            deep_work_threshold_mins,
            session_stats: Vec::new(),
        }
    }

    /// Process a session and extract metrics.
    pub fn process_session(&mut self, session: &Session) {
        let date = session.created_at.date_naive();

        let summary = self
            .daily_summaries
            .entry(date)
            .or_insert_with(|| DailySummary::new(date));
        let day_tools = self.daily_tool_counts.entry(date).or_default();

        summary.session_count += 1;
        summary.message_count += session.messages.len() as u32;

        // Merge session tags.
        for tag in &session.tags {
            if !summary.tags.contains(tag) {
                summary.tags.push(tag.clone());
            }
        }

        // Calculate session duration from first to last message timestamp.
        let duration_secs = session_duration(session);
        summary.total_active_time_secs += duration_secs;

        let mut stats = SessionStats {
            session_id: session.id.clone(),
            session_name: session.name.clone(),
            date,
            duration_secs,
            message_count: session.messages.len() as u32,
            user_message_count: 0,
            assistant_message_count: 0,
            tool_call_count: 0,
            tool_error_count: 0,
            tools_used: Vec::new(),
        };

        for msg in &session.messages {
            match msg.role {
                Role::User => {
                    summary.user_message_count += 1;
                    stats.user_message_count += 1;
                }
                Role::Assistant => {
                    summary.assistant_message_count += 1;
                    stats.assistant_message_count += 1;

                    // Count tool calls from assistant messages.
                    if let Some(calls) = &msg.tool_calls {
                        for call in calls {
                            summary.tool_call_count += 1;
                            stats.tool_call_count += 1;

                            *self.tool_counts.entry(call.name.clone()).or_insert(0) += 1;
                            *day_tools.entry(call.name.clone()).or_insert(0) += 1;

                            if !stats.tools_used.contains(&call.name) {
                                stats.tools_used.push(call.name.clone());
                            }
                        }
                    }
                }
                Role::Tool => {
                    // Check for tool errors by looking for error indicators.
                    // Tool results that start with "Error" or contain is_error pattern.
                    if msg.content.starts_with("Error") || msg.content.starts_with("error:") {
                        summary.tool_error_count += 1;
                        stats.tool_error_count += 1;
                    }
                }
                Role::System => {}
            }
        }

        self.session_stats.push(stats);
    }

    /// Process multiple sessions.
    pub fn process_sessions(&mut self, sessions: &[Session]) {
        for session in sessions {
            self.process_session(session);
        }
        self.finalize_all();
    }

    /// Finalize all daily summaries (compute top tools).
    pub fn finalize_all(&mut self) {
        let dates: Vec<NaiveDate> = self.daily_tool_counts.keys().cloned().collect();
        for date in dates {
            self.finalize_summary(date);
        }
    }

    /// Finalize a daily summary: set top_tools from accumulated day counts.
    pub fn finalize_summary(&mut self, date: NaiveDate) {
        if let Some(day_tools) = self.daily_tool_counts.get(&date) {
            let mut sorted: Vec<(String, u32)> =
                day_tools.iter().map(|(k, v)| (k.clone(), *v)).collect();
            sorted.sort_by(|a, b| b.1.cmp(&a.1));
            sorted.truncate(10);

            if let Some(summary) = self.daily_summaries.get_mut(&date) {
                summary.top_tools = sorted;
            }
        }
    }

    /// Get daily summary for a specific date.
    pub fn get_daily_summary(&self, date: NaiveDate) -> Option<&DailySummary> {
        self.daily_summaries.get(&date)
    }

    /// Get summaries for a date range, sorted chronologically.
    pub fn get_range_summaries(&self, start: NaiveDate, end: NaiveDate) -> Vec<&DailySummary> {
        let mut summaries: Vec<_> = self
            .daily_summaries
            .iter()
            .filter(|(date, _)| **date >= start && **date <= end)
            .map(|(_, s)| s)
            .collect();
        summaries.sort_by_key(|s| s.date);
        summaries
    }

    /// Get top tools across all processed sessions.
    pub fn top_tools(&self, limit: usize) -> Vec<(String, u32)> {
        let mut counts: Vec<_> = self
            .tool_counts
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();
        counts.sort_by(|a, b| b.1.cmp(&a.1));
        counts.truncate(limit);
        counts
    }

    /// Total active time in a date range (seconds).
    pub fn total_active_time(&self, start: NaiveDate, end: NaiveDate) -> u64 {
        self.daily_summaries
            .iter()
            .filter(|(date, _)| **date >= start && **date <= end)
            .map(|(_, s)| s.total_active_time_secs)
            .sum()
    }

    /// Detect deep work sessions (duration >= threshold).
    pub fn deep_work_sessions(&self) -> Vec<&SessionStats> {
        let threshold_secs = self.deep_work_threshold_mins as u64 * 60;
        self.session_stats
            .iter()
            .filter(|s| s.duration_secs >= threshold_secs)
            .collect()
    }

    /// Calculate average session duration across all processed sessions.
    pub fn average_session_duration(&self) -> Option<u64> {
        if self.session_stats.is_empty() {
            return None;
        }
        let total: u64 = self.session_stats.iter().map(|s| s.duration_secs).sum();
        Some(total / self.session_stats.len() as u64)
    }

    /// Tool error rate for a date range (0.0..1.0).
    pub fn error_rate(&self, start: NaiveDate, end: NaiveDate) -> f64 {
        let summaries = self.get_range_summaries(start, end);
        let total_calls: u32 = summaries.iter().map(|s| s.tool_call_count).sum();
        let total_errors: u32 = summaries.iter().map(|s| s.tool_error_count).sum();

        if total_calls == 0 {
            0.0
        } else {
            total_errors as f64 / total_calls as f64
        }
    }

    /// Get all processed session stats.
    pub fn session_stats(&self) -> &[SessionStats] {
        &self.session_stats
    }

    /// Total number of sessions processed.
    pub fn total_sessions(&self) -> usize {
        self.session_stats.len()
    }

    /// Total number of active days.
    pub fn active_days(&self) -> usize {
        self.daily_summaries.len()
    }
}

impl Default for Analytics {
    fn default() -> Self {
        Self::new(30)
    }
}

/// Calculate session duration from first to last message timestamp (seconds).
fn session_duration(session: &Session) -> u64 {
    if session.messages.len() < 2 {
        return 0;
    }

    let first = session.messages.first().map(|m| m.timestamp);
    let last = session.messages.last().map(|m| m.timestamp);

    match (first, last) {
        (Some(f), Some(l)) => {
            let diff = l - f;
            diff.num_seconds().max(0) as u64
        }
        _ => 0,
    }
}

/// Format seconds as a human-readable duration string.
pub fn format_duration(seconds: u64) -> String {
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;

    if hours > 0 {
        format!("{}h {}m", hours, minutes)
    } else {
        format!("{}m", minutes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::types::{Message, ToolCall};
    use chrono::Duration;

    fn make_session(name: &str, messages: Vec<Message>) -> Session {
        let mut session = Session::new(name);
        session.messages = messages;
        session
    }

    fn user_msg(content: &str, offset_secs: i64) -> Message {
        let mut msg = Message::user(content);
        msg.timestamp = chrono::Utc::now() + Duration::seconds(offset_secs);
        msg
    }

    fn assistant_msg(content: &str, offset_secs: i64) -> Message {
        let mut msg = Message::assistant(content);
        msg.timestamp = chrono::Utc::now() + Duration::seconds(offset_secs);
        msg
    }

    fn assistant_with_tool(tool_name: &str, offset_secs: i64) -> Message {
        let mut msg = Message::assistant_with_tool_calls(
            "",
            vec![ToolCall {
                id: "tc-1".into(),
                name: tool_name.into(),
                arguments: "{}".into(),
            }],
        );
        msg.timestamp = chrono::Utc::now() + Duration::seconds(offset_secs);
        msg
    }

    fn tool_result(content: &str, offset_secs: i64) -> Message {
        let mut msg = Message::tool_result("tc-1", content);
        msg.timestamp = chrono::Utc::now() + Duration::seconds(offset_secs);
        msg
    }

    #[test]
    fn test_empty_analytics() {
        let analytics = Analytics::default();
        assert_eq!(analytics.total_sessions(), 0);
        assert_eq!(analytics.active_days(), 0);
        assert!(analytics.average_session_duration().is_none());
    }

    #[test]
    fn test_process_single_session() {
        let mut analytics = Analytics::default();

        let session = make_session(
            "test",
            vec![
                user_msg("hello", 0),
                assistant_msg("hi there", 5),
                user_msg("help me", 10),
                assistant_msg("sure", 15),
            ],
        );

        analytics.process_session(&session);
        analytics.finalize_all();

        assert_eq!(analytics.total_sessions(), 1);
        assert_eq!(analytics.active_days(), 1);

        let today = chrono::Utc::now().date_naive();
        let summary = analytics.get_daily_summary(today).unwrap();
        assert_eq!(summary.session_count, 1);
        assert_eq!(summary.message_count, 4);
        assert_eq!(summary.user_message_count, 2);
        assert_eq!(summary.assistant_message_count, 2);
    }

    #[test]
    fn test_tool_call_counting() {
        let mut analytics = Analytics::default();

        let session = make_session(
            "tools",
            vec![
                user_msg("do something", 0),
                assistant_with_tool("shell_exec", 5),
                tool_result("done", 10),
                assistant_with_tool("file_read", 15),
                tool_result("contents", 20),
                assistant_with_tool("shell_exec", 25),
                tool_result("Error: command failed", 30),
                assistant_msg("I ran three tools", 35),
            ],
        );

        analytics.process_session(&session);
        analytics.finalize_all();

        let today = chrono::Utc::now().date_naive();
        let summary = analytics.get_daily_summary(today).unwrap();
        assert_eq!(summary.tool_call_count, 3);
        assert_eq!(summary.tool_error_count, 1);

        // Top tools should include shell_exec(2), file_read(1).
        let top = analytics.top_tools(10);
        assert_eq!(top[0], ("shell_exec".to_string(), 2));
        assert_eq!(top[1], ("file_read".to_string(), 1));
    }

    #[test]
    fn test_session_duration() {
        let session = make_session("dur", vec![user_msg("a", 0), assistant_msg("b", 120)]);
        let dur = session_duration(&session);
        // Should be approximately 120 seconds.
        assert!((119..=121).contains(&dur));
    }

    #[test]
    fn test_session_duration_single_message() {
        let session = make_session("single", vec![user_msg("a", 0)]);
        assert_eq!(session_duration(&session), 0);
    }

    #[test]
    fn test_session_duration_empty() {
        let session = make_session("empty", vec![]);
        assert_eq!(session_duration(&session), 0);
    }

    #[test]
    fn test_deep_work_sessions() {
        let mut analytics = Analytics::new(5); // 5 minute threshold.

        let short = make_session("short", vec![user_msg("hi", 0), assistant_msg("bye", 60)]);
        let long = make_session(
            "long",
            vec![user_msg("start", 0), assistant_msg("end", 600)],
        );

        analytics.process_session(&short);
        analytics.process_session(&long);

        let deep = analytics.deep_work_sessions();
        assert_eq!(deep.len(), 1);
        assert_eq!(deep[0].session_name, "long");
    }

    #[test]
    fn test_average_session_duration() {
        let mut analytics = Analytics::default();

        let s1 = make_session("s1", vec![user_msg("a", 0), assistant_msg("b", 100)]);
        let s2 = make_session("s2", vec![user_msg("a", 0), assistant_msg("b", 200)]);

        analytics.process_session(&s1);
        analytics.process_session(&s2);

        let avg = analytics.average_session_duration().unwrap();
        // Average of ~100 and ~200.
        assert!((149..=151).contains(&avg));
    }

    #[test]
    fn test_error_rate() {
        let mut analytics = Analytics::default();

        let session = make_session(
            "errs",
            vec![
                user_msg("go", 0),
                assistant_with_tool("shell_exec", 5),
                tool_result("ok", 10),
                assistant_with_tool("shell_exec", 15),
                tool_result("Error: fail", 20),
            ],
        );

        analytics.process_session(&session);
        let today = chrono::Utc::now().date_naive();
        let rate = analytics.error_rate(today, today);
        assert!((rate - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_date_range_summaries() {
        let mut analytics = Analytics::default();

        let session = make_session("today", vec![user_msg("hi", 0), assistant_msg("hello", 5)]);
        analytics.process_session(&session);

        let today = chrono::Utc::now().date_naive();
        let yesterday = today - Duration::days(1);

        // Range includes today.
        let summaries = analytics.get_range_summaries(yesterday, today);
        assert_eq!(summaries.len(), 1);

        // Range before today.
        let old = today - Duration::days(10);
        let summaries = analytics.get_range_summaries(old, yesterday);
        assert!(summaries.is_empty());
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(0), "0m");
        assert_eq!(format_duration(59), "0m");
        assert_eq!(format_duration(60), "1m");
        assert_eq!(format_duration(3600), "1h 0m");
        assert_eq!(format_duration(3661), "1h 1m");
        assert_eq!(format_duration(7200), "2h 0m");
        assert_eq!(format_duration(5400), "1h 30m");
    }

    #[test]
    fn test_daily_summary_error_rate() {
        let mut summary = DailySummary::new(chrono::Utc::now().date_naive());
        assert_eq!(summary.tool_error_rate(), 0.0);

        summary.tool_call_count = 10;
        summary.tool_error_count = 3;
        assert!((summary.tool_error_rate() - 0.3).abs() < 0.001);
    }

    #[test]
    fn test_session_tags_aggregation() {
        let mut analytics = Analytics::default();

        let mut s1 = Session::new("s1");
        s1.add_tag("rust");
        s1.add_tag("debug");
        s1.messages.push(Message::user("hi"));

        let mut s2 = Session::new("s2");
        s2.add_tag("rust");
        s2.add_tag("feature");
        s2.messages.push(Message::user("hello"));

        analytics.process_session(&s1);
        analytics.process_session(&s2);

        let today = chrono::Utc::now().date_naive();
        let summary = analytics.get_daily_summary(today).unwrap();
        assert!(summary.tags.contains(&"rust".to_string()));
        assert!(summary.tags.contains(&"debug".to_string()));
        assert!(summary.tags.contains(&"feature".to_string()));
        assert_eq!(summary.tags.len(), 3); // No duplicates.
    }

    #[test]
    fn test_session_stats() {
        let mut analytics = Analytics::default();

        let session = make_session(
            "test-stats",
            vec![
                user_msg("go", 0),
                assistant_with_tool("shell_exec", 5),
                tool_result("ok", 10),
                assistant_msg("done", 15),
            ],
        );

        analytics.process_session(&session);

        let stats = analytics.session_stats();
        assert_eq!(stats.len(), 1);
        assert_eq!(stats[0].session_name, "test-stats");
        assert_eq!(stats[0].tool_call_count, 1);
        assert_eq!(stats[0].tools_used, vec!["shell_exec"]);
    }

    #[test]
    fn test_process_multiple_sessions() {
        let mut analytics = Analytics::default();

        let sessions = vec![
            make_session("s1", vec![user_msg("a", 0), assistant_msg("b", 10)]),
            make_session("s2", vec![user_msg("c", 0), assistant_msg("d", 20)]),
            make_session("s3", vec![user_msg("e", 0), assistant_msg("f", 30)]),
        ];

        analytics.process_sessions(&sessions);

        assert_eq!(analytics.total_sessions(), 3);
        let today = chrono::Utc::now().date_naive();
        let summary = analytics.get_daily_summary(today).unwrap();
        assert_eq!(summary.session_count, 3);
        assert_eq!(summary.message_count, 6);
    }
}
