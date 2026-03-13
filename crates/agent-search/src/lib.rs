//! Full-text search over terminal sessions and commands.
//!
//! Wraps [Tantivy](https://docs.rs/tantivy) to provide indexing and querying
//! of [`TerminalSession`](agent_core::terminal_session::TerminalSession) and
//! [`TerminalCommand`](agent_core::terminal_session::TerminalCommand) records.
//! Ported from ShellVault's `search` module and adapted to the Agent-Shell
//! type system.

pub mod indexer;
pub mod query;

pub use indexer::{SearchHit, SearchIndexer};
pub use query::SearchQuery;

use thiserror::Error;

/// Errors that can occur during search operations.
#[derive(Debug, Error)]
pub enum SearchError {
    #[error("tantivy error: {0}")]
    Tantivy(#[from] tantivy::TantivyError),

    #[error("query parse error: {0}")]
    QueryParse(#[from] tantivy::query::QueryParserError),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("index not found at {0}")]
    IndexNotFound(String),
}

/// Convenience alias used throughout this crate.
pub type SearchResult<T> = std::result::Result<T, SearchError>;
