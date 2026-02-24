//! Skill system for agent-shell.
//!
//! Provides skill indexing, full-text search, metadata validation,
//! and file-watching for automatic reloading.
//!
//! Ported from Skill-MCP-Claude.

pub mod indexer;
pub mod models;
pub mod search;
pub mod validation;
pub mod watcher;

pub use indexer::{IndexError, SkillIndexer};
pub use models::{
    ContentIndex, ContentIndexEntry, MatchType, SearchOptions, SearchResult, SearchResults,
    SkillContent, SkillIndex, SkillMeta, SubSkillContent, SubSkillMeta, ValidationResult,
};
pub use search::SearchService;
pub use validation::{validate_meta, validate_skills};
pub use watcher::{FileWatcher, WatchError};
