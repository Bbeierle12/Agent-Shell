//! Analytics, aggregation, and reporting for agent-shell sessions.
//!
//! Computes metrics from session data: daily summaries, tool usage frequency,
//! conversation patterns, and markdown report generation.

pub mod aggregations;
pub mod reports;

pub use aggregations::{Analytics, DailySummary, SessionStats};
pub use reports::ReportGenerator;
