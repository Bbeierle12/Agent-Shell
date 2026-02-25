//! Markdown report generation from analytics data.
//!
//! Generates weekly, monthly, and summary reports in markdown format.

use crate::aggregations::{format_duration, Analytics};
use chrono::{Datelike, Duration, NaiveDate};

/// Report generator for creating markdown summaries.
pub struct ReportGenerator;

impl ReportGenerator {
    /// Generate a weekly report.
    pub fn weekly_report(analytics: &Analytics, week_start: NaiveDate) -> String {
        let week_end = week_start + Duration::days(6);
        let summaries = analytics.get_range_summaries(week_start, week_end);

        let mut report = String::new();

        report.push_str(&format!(
            "# Weekly Report\n\n**{} - {}**\n\n",
            week_start.format("%B %d, %Y"),
            week_end.format("%B %d, %Y")
        ));

        // Overview.
        let total_time = analytics.total_active_time(week_start, week_end);
        let total_sessions: u32 = summaries.iter().map(|s| s.session_count).sum();
        let total_messages: u32 = summaries.iter().map(|s| s.message_count).sum();
        let total_tool_calls: u32 = summaries.iter().map(|s| s.tool_call_count).sum();
        let total_tool_errors: u32 = summaries.iter().map(|s| s.tool_error_count).sum();

        report.push_str("## Overview\n\n");
        report.push_str(&format!(
            "- **Active Time:** {}\n",
            format_duration(total_time)
        ));
        report.push_str(&format!("- **Sessions:** {}\n", total_sessions));
        report.push_str(&format!("- **Messages:** {}\n", total_messages));
        report.push_str(&format!("- **Tool Calls:** {}\n", total_tool_calls));
        report.push_str(&format!(
            "- **Tool Errors:** {} ({:.1}%)\n\n",
            total_tool_errors,
            if total_tool_calls > 0 {
                total_tool_errors as f64 / total_tool_calls as f64 * 100.0
            } else {
                0.0
            }
        ));

        // Daily breakdown table.
        report.push_str("## Daily Breakdown\n\n");
        report.push_str("| Day | Active Time | Sessions | Messages | Tool Calls |\n");
        report.push_str("|-----|-------------|----------|----------|------------|\n");

        let mut current_date = week_start;
        while current_date <= week_end {
            let day_name = current_date.format("%A");
            if let Some(summary) = analytics.get_daily_summary(current_date) {
                report.push_str(&format!(
                    "| {} | {} | {} | {} | {} |\n",
                    day_name,
                    format_duration(summary.total_active_time_secs),
                    summary.session_count,
                    summary.message_count,
                    summary.tool_call_count,
                ));
            } else {
                report.push_str(&format!("| {} | - | 0 | 0 | 0 |\n", day_name));
            }
            current_date += Duration::days(1);
        }
        report.push('\n');

        // Top tools.
        let top_tools = analytics.top_tools(10);
        if !top_tools.is_empty() {
            report.push_str("## Top Tools\n\n");
            for (i, (tool, count)) in top_tools.iter().enumerate() {
                report.push_str(&format!("{}. `{}` - {} calls\n", i + 1, tool, count));
            }
            report.push('\n');
        }

        // Tags.
        let mut all_tags: Vec<String> = summaries
            .iter()
            .flat_map(|s| s.tags.iter().cloned())
            .collect();
        all_tags.sort();
        all_tags.dedup();

        if !all_tags.is_empty() {
            report.push_str("## Tags\n\n");
            for tag in &all_tags {
                report.push_str(&format!("- {}\n", tag));
            }
            report.push('\n');
        }

        report
    }

    /// Generate a monthly report.
    pub fn monthly_report(analytics: &Analytics, year: i32, month: u32) -> String {
        let first_day = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
        let last_day = if month == 12 {
            NaiveDate::from_ymd_opt(year + 1, 1, 1).unwrap() - Duration::days(1)
        } else {
            NaiveDate::from_ymd_opt(year, month + 1, 1).unwrap() - Duration::days(1)
        };

        let summaries = analytics.get_range_summaries(first_day, last_day);

        let mut report = String::new();

        report.push_str(&format!(
            "# Monthly Report\n\n**{}**\n\n",
            first_day.format("%B %Y")
        ));

        // Overview.
        let total_time = analytics.total_active_time(first_day, last_day);
        let total_sessions: u32 = summaries.iter().map(|s| s.session_count).sum();
        let total_messages: u32 = summaries.iter().map(|s| s.message_count).sum();
        let active_days = summaries.len();

        report.push_str("## Overview\n\n");
        report.push_str(&format!(
            "- **Active Time:** {}\n",
            format_duration(total_time)
        ));
        report.push_str(&format!(
            "- **Active Days:** {} / {}\n",
            active_days,
            last_day.day()
        ));
        report.push_str(&format!("- **Sessions:** {}\n", total_sessions));
        report.push_str(&format!("- **Messages:** {}\n", total_messages));
        report.push_str(&format!(
            "- **Avg Daily Time:** {}\n\n",
            if active_days > 0 {
                format_duration(total_time / active_days as u64)
            } else {
                "0m".to_string()
            }
        ));

        // Weekly breakdown table.
        report.push_str("## Weekly Breakdown\n\n");
        report.push_str("| Week | Active Time | Sessions | Messages |\n");
        report.push_str("|------|-------------|----------|----------|\n");

        let mut week_num = 1;
        let mut current_date = first_day;
        while current_date <= last_day {
            let week_end = std::cmp::min(current_date + Duration::days(6), last_day);

            let week_summaries = analytics.get_range_summaries(current_date, week_end);
            let week_time: u64 = week_summaries
                .iter()
                .map(|s| s.total_active_time_secs)
                .sum();
            let week_sessions: u32 = week_summaries.iter().map(|s| s.session_count).sum();
            let week_messages: u32 = week_summaries.iter().map(|s| s.message_count).sum();

            report.push_str(&format!(
                "| Week {} | {} | {} | {} |\n",
                week_num,
                format_duration(week_time),
                week_sessions,
                week_messages,
            ));

            current_date = week_end + Duration::days(1);
            week_num += 1;
        }
        report.push('\n');

        // Top tools.
        let top_tools = analytics.top_tools(15);
        if !top_tools.is_empty() {
            report.push_str("## Top Tools\n\n");
            for (i, (tool, count)) in top_tools.iter().enumerate() {
                report.push_str(&format!("{}. `{}` - {} calls\n", i + 1, tool, count));
            }
            report.push('\n');
        }

        report
    }

    /// Generate a compact summary suitable for display in the REPL.
    pub fn text_summary(analytics: &Analytics) -> String {
        let mut output = String::new();

        let today = chrono::Utc::now().date_naive();
        let week_ago = today - Duration::days(7);

        // Today.
        if let Some(summary) = analytics.get_daily_summary(today) {
            output.push_str("  Today:\n");
            output.push_str(&format!(
                "    Sessions: {}  Messages: {}  Time: {}\n",
                summary.session_count,
                summary.message_count,
                format_duration(summary.total_active_time_secs)
            ));
            if summary.tool_call_count > 0 {
                output.push_str(&format!(
                    "    Tool calls: {}  Errors: {}\n",
                    summary.tool_call_count, summary.tool_error_count
                ));
            }
        } else {
            output.push_str("  Today: no activity\n");
        }

        // This week.
        let week_summaries = analytics.get_range_summaries(week_ago, today);
        let week_time: u64 = week_summaries
            .iter()
            .map(|s| s.total_active_time_secs)
            .sum();
        let week_sessions: u32 = week_summaries.iter().map(|s| s.session_count).sum();
        let week_messages: u32 = week_summaries.iter().map(|s| s.message_count).sum();

        output.push_str(&format!(
            "  This week: {} sessions, {} messages, {}\n",
            week_sessions,
            week_messages,
            format_duration(week_time)
        ));

        // All time.
        output.push_str(&format!(
            "  All time: {} sessions across {} days\n",
            analytics.total_sessions(),
            analytics.active_days()
        ));

        if let Some(avg) = analytics.average_session_duration() {
            output.push_str(&format!("  Avg session: {}\n", format_duration(avg)));
        }

        // Top tools.
        let top = analytics.top_tools(5);
        if !top.is_empty() {
            output.push_str("  Top tools:");
            for (tool, count) in &top {
                output.push_str(&format!(" {}({})", tool, count));
            }
            output.push('\n');
        }

        // Deep work.
        let deep = analytics.deep_work_sessions();
        if !deep.is_empty() {
            output.push_str(&format!("  Deep work sessions (30m+): {}\n", deep.len()));
        }

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregations::Analytics;
    use agent_core::types::Message;

    fn make_session_on_date(
        name: &str,
        date: NaiveDate,
        messages: Vec<Message>,
    ) -> agent_core::session::Session {
        let mut session = agent_core::session::Session::new(name);
        session.created_at = date.and_hms_opt(10, 0, 0).unwrap().and_utc();
        session.messages = messages;
        session
    }

    fn user_msg_at(content: &str, date: NaiveDate, hour: u32) -> Message {
        let mut msg = Message::user(content);
        msg.timestamp = date.and_hms_opt(hour, 0, 0).unwrap().and_utc();
        msg
    }

    fn assistant_msg_at(content: &str, date: NaiveDate, hour: u32) -> Message {
        let mut msg = Message::assistant(content);
        msg.timestamp = date.and_hms_opt(hour, 0, 0).unwrap().and_utc();
        msg
    }

    #[test]
    fn test_weekly_report_structure() {
        let mut analytics = Analytics::default();
        let today = chrono::Utc::now().date_naive();
        let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);

        let session = make_session_on_date(
            "test",
            monday,
            vec![
                user_msg_at("hi", monday, 10),
                assistant_msg_at("hello", monday, 11),
            ],
        );
        analytics.process_session(&session);
        analytics.finalize_all();

        let report = ReportGenerator::weekly_report(&analytics, monday);
        assert!(report.contains("# Weekly Report"));
        assert!(report.contains("## Overview"));
        assert!(report.contains("## Daily Breakdown"));
        assert!(report.contains("Sessions"));
    }

    #[test]
    fn test_monthly_report_structure() {
        let mut analytics = Analytics::default();
        let today = chrono::Utc::now().date_naive();

        let session = make_session_on_date(
            "test",
            today,
            vec![
                user_msg_at("hi", today, 10),
                assistant_msg_at("hello", today, 11),
            ],
        );
        analytics.process_session(&session);
        analytics.finalize_all();

        let report = ReportGenerator::monthly_report(&analytics, today.year(), today.month());
        assert!(report.contains("# Monthly Report"));
        assert!(report.contains("## Overview"));
        assert!(report.contains("## Weekly Breakdown"));
    }

    #[test]
    fn test_text_summary() {
        let mut analytics = Analytics::default();

        let today = chrono::Utc::now().date_naive();
        let session = make_session_on_date(
            "test",
            today,
            vec![
                user_msg_at("hi", today, 10),
                assistant_msg_at("hello", today, 11),
            ],
        );
        analytics.process_session(&session);
        analytics.finalize_all();

        let summary = ReportGenerator::text_summary(&analytics);
        assert!(summary.contains("Today:"));
        assert!(summary.contains("This week:"));
        assert!(summary.contains("All time:"));
    }

    #[test]
    fn test_empty_weekly_report() {
        let analytics = Analytics::default();
        let today = chrono::Utc::now().date_naive();
        let report = ReportGenerator::weekly_report(&analytics, today);
        assert!(report.contains("# Weekly Report"));
        assert!(report.contains("0 |")); // Table rows should show zeros.
    }

    #[test]
    fn test_empty_text_summary() {
        let analytics = Analytics::default();
        let summary = ReportGenerator::text_summary(&analytics);
        assert!(summary.contains("no activity"));
        assert!(summary.contains("0 sessions"));
    }
}
