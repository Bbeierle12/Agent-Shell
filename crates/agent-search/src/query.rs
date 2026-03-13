//! Search query DSL parser.
//!
//! Parses human-friendly query strings into a structured [`SearchQuery`] that
//! the indexer can convert to a Tantivy query.
//!
//! Syntax examples:
//! ```text
//! cargo test                         # free-text
//! cargo test project:myapp           # with project/directory filter
//! git exit:0 after:2024-01-01        # exit code + date range
//! program:docker category:Container  # exact program/category match
//! ```

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

/// Structured search query with optional filters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SearchQuery {
    /// Free-text search terms matched against command text, directory, and output.
    pub text: String,
    /// Exact program name filter (e.g. `git`, `cargo`).
    pub program: Option<String>,
    /// Tool category filter (e.g. `VersionControl`, `Container`).
    pub category: Option<String>,
    /// Directory substring filter.
    pub directory: Option<String>,
    /// Only return commands executed after this timestamp.
    pub after: Option<DateTime<Utc>>,
    /// Only return commands executed before this timestamp.
    pub before: Option<DateTime<Utc>>,
    /// Exact exit code filter.
    pub exit_code: Option<i32>,
}

impl SearchQuery {
    /// Parse a human-readable query string into a [`SearchQuery`].
    ///
    /// Recognised filter prefixes (case-insensitive):
    ///
    /// | Prefix | Aliases | Example |
    /// |--------|---------|---------|
    /// | `program:` | `prog:`, `p:` | `program:git` |
    /// | `category:` | `cat:` | `category:Container` |
    /// | `exit:` | `code:` | `exit:0` |
    /// | `after:` | `from:`, `since:` | `after:2024-01-01` |
    /// | `before:` | `until:`, `to:` | `before:2025-12-31` |
    /// | `dir:` | `directory:`, `path:` | `dir:/home/user` |
    ///
    /// Everything else is treated as free-text search terms.
    pub fn parse(input: &str) -> Self {
        let mut result = Self::default();
        let mut text_parts: Vec<&str> = Vec::new();

        for token in input.split_whitespace() {
            if let Some((key, value)) = token.split_once(':') {
                match key.to_lowercase().as_str() {
                    "program" | "prog" | "p" => {
                        result.program = Some(value.to_string());
                    }
                    "category" | "cat" => {
                        result.category = Some(value.to_string());
                    }
                    "exit" | "code" => {
                        if let Ok(code) = value.parse() {
                            result.exit_code = Some(code);
                        }
                    }
                    "after" | "from" | "since" => {
                        if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
                            result.after =
                                Some(date.and_hms_opt(0, 0, 0).unwrap().and_utc());
                        }
                    }
                    "before" | "until" | "to" => {
                        if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
                            result.before =
                                Some(date.and_hms_opt(23, 59, 59).unwrap().and_utc());
                        }
                    }
                    "dir" | "directory" | "path" => {
                        result.directory = Some(value.to_string());
                    }
                    _ => {
                        // Unknown prefix -- treat the whole token as text.
                        text_parts.push(token);
                    }
                }
            } else {
                text_parts.push(token);
            }
        }

        result.text = text_parts.join(" ");
        result
    }

    /// Returns `true` when no filters or text are set.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
            && self.program.is_none()
            && self.category.is_none()
            && self.directory.is_none()
            && self.after.is_none()
            && self.before.is_none()
            && self.exit_code.is_none()
    }

    /// Returns `true` when at least one structured filter is present
    /// (ignoring free-text).
    pub fn has_filters(&self) -> bool {
        self.program.is_some()
            || self.category.is_some()
            || self.directory.is_some()
            || self.after.is_some()
            || self.before.is_some()
            || self.exit_code.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_text() {
        let q = SearchQuery::parse("cargo test --lib");
        assert_eq!(q.text, "cargo test --lib");
        assert!(!q.has_filters());
    }

    #[test]
    fn test_parse_program_filter() {
        let q = SearchQuery::parse("status program:git");
        assert_eq!(q.text, "status");
        assert_eq!(q.program, Some("git".into()));
    }

    #[test]
    fn test_parse_program_alias() {
        let q = SearchQuery::parse("p:cargo build");
        assert_eq!(q.program, Some("cargo".into()));
        assert_eq!(q.text, "build");
    }

    #[test]
    fn test_parse_category_filter() {
        let q = SearchQuery::parse("cat:VersionControl");
        assert_eq!(q.category, Some("VersionControl".into()));
        assert!(q.text.is_empty());
    }

    #[test]
    fn test_parse_exit_code() {
        let q = SearchQuery::parse("exit:0");
        assert_eq!(q.exit_code, Some(0));

        let q2 = SearchQuery::parse("code:127");
        assert_eq!(q2.exit_code, Some(127));
    }

    #[test]
    fn test_parse_date_filters() {
        let q = SearchQuery::parse("after:2024-01-01 before:2024-12-31");
        assert!(q.after.is_some());
        assert!(q.before.is_some());

        let after = q.after.unwrap();
        assert_eq!(after.date_naive().to_string(), "2024-01-01");

        let before = q.before.unwrap();
        assert_eq!(before.date_naive().to_string(), "2024-12-31");
    }

    #[test]
    fn test_parse_date_aliases() {
        let q = SearchQuery::parse("since:2024-06-01 until:2024-07-01");
        assert!(q.after.is_some());
        assert!(q.before.is_some());
    }

    #[test]
    fn test_parse_directory_filter() {
        let q = SearchQuery::parse("dir:/home/user/project");
        assert_eq!(q.directory, Some("/home/user/project".into()));
    }

    #[test]
    fn test_parse_directory_aliases() {
        for prefix in ["dir", "directory", "path"] {
            let q = SearchQuery::parse(&format!("{}:/tmp", prefix));
            assert_eq!(q.directory, Some("/tmp".into()), "failed for prefix {}", prefix);
        }
    }

    #[test]
    fn test_parse_complex_query() {
        let q = SearchQuery::parse("cargo test program:cargo exit:0 after:2024-01-01 dir:/home/user");
        assert_eq!(q.text, "cargo test");
        assert_eq!(q.program, Some("cargo".into()));
        assert_eq!(q.exit_code, Some(0));
        assert!(q.after.is_some());
        assert_eq!(q.directory, Some("/home/user".into()));
        assert!(q.has_filters());
    }

    #[test]
    fn test_parse_unknown_prefix_treated_as_text() {
        let q = SearchQuery::parse("foo:bar baz");
        assert_eq!(q.text, "foo:bar baz");
        assert!(!q.has_filters());
    }

    #[test]
    fn test_parse_empty_string() {
        let q = SearchQuery::parse("");
        assert!(q.is_empty());
    }

    #[test]
    fn test_parse_invalid_date_ignored() {
        let q = SearchQuery::parse("after:not-a-date");
        assert!(q.after.is_none());
        // The token is discarded (known prefix, just invalid value).
        assert!(q.text.is_empty());
    }

    #[test]
    fn test_parse_invalid_exit_code_ignored() {
        let q = SearchQuery::parse("exit:abc");
        assert!(q.exit_code.is_none());
    }

    #[test]
    fn test_is_empty() {
        assert!(SearchQuery::default().is_empty());

        let mut q = SearchQuery::default();
        q.text = "hello".into();
        assert!(!q.is_empty());

        let mut q2 = SearchQuery::default();
        q2.exit_code = Some(0);
        assert!(!q2.is_empty());
    }
}
