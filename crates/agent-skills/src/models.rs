//! Core data models for the skill system.
//!
//! Ported from Skill-MCP-Claude's Rust models.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ── Skill Metadata ──────────────────────────────────────────────────────

/// Sub-skill reference within a parent skill.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubSkillMeta {
    /// Sub-skill identifier (e.g., "validation", "react").
    pub name: String,

    /// Relative path to the sub-skill markdown file.
    pub file: String,

    /// Optional keywords for search discovery.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<String>,
}

/// Primary skill metadata from `_meta.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillMeta {
    /// Skill identifier — must match directory name.
    /// Lowercase alphanumeric with hyphens only.
    pub name: String,

    /// Human-readable description of what the skill provides.
    pub description: String,

    /// Optional search tags for discovery.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// Optional nested sub-skills for domain/router skills.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_skills: Option<Vec<SubSkillMeta>>,

    /// Optional origin indicator (e.g., "community", "official").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl SkillMeta {
    /// Check if this skill has sub-skills (is a router/domain skill).
    pub fn has_sub_skills(&self) -> bool {
        self.sub_skills
            .as_ref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    /// Get sub-skill names if any.
    pub fn sub_skill_names(&self) -> Vec<&str> {
        self.sub_skills
            .as_ref()
            .map(|subs| subs.iter().map(|s| s.name.as_str()).collect())
            .unwrap_or_default()
    }

    /// Find a sub-skill by name.
    pub fn find_sub_skill(&self, name: &str) -> Option<&SubSkillMeta> {
        self.sub_skills
            .as_ref()
            .and_then(|subs| subs.iter().find(|s| s.name == name))
    }

    /// Get all trigger words (skill-level tags + sub-skill triggers).
    pub fn all_triggers(&self) -> Vec<&str> {
        let mut triggers: Vec<&str> = self.tags.iter().map(|s| s.as_str()).collect();

        if let Some(subs) = &self.sub_skills {
            for sub in subs {
                triggers.extend(sub.triggers.iter().map(|s| s.as_str()));
            }
        }

        triggers
    }
}

// ── Skill Index ─────────────────────────────────────────────────────────

/// Aggregated skill metadata index.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillIndex {
    /// All loaded skill metadata.
    pub skills: Vec<SkillMeta>,

    /// Errors encountered during index building.
    #[serde(default)]
    pub validation_errors: Vec<String>,

    /// ISO timestamp of last index update.
    pub last_updated: DateTime<Utc>,
}

impl SkillIndex {
    /// Create a new empty index.
    pub fn new() -> Self {
        Self {
            skills: Vec::new(),
            validation_errors: Vec::new(),
            last_updated: Utc::now(),
        }
    }

    /// Create index with skills and errors.
    pub fn with_skills(skills: Vec<SkillMeta>, errors: Vec<String>) -> Self {
        Self {
            skills,
            validation_errors: errors,
            last_updated: Utc::now(),
        }
    }

    /// Find a skill by name.
    pub fn find(&self, name: &str) -> Option<&SkillMeta> {
        self.skills.iter().find(|s| s.name == name)
    }

    /// Get skill count.
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    /// Check if index is empty.
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// Check if there were validation errors.
    pub fn has_errors(&self) -> bool {
        !self.validation_errors.is_empty()
    }
}

impl Default for SkillIndex {
    fn default() -> Self {
        Self::new()
    }
}

// ── Content Index ───────────────────────────────────────────────────────

/// Single entry in the content index for full-text search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentIndexEntry {
    /// Parent skill domain.
    pub domain: String,

    /// Sub-skill name if this is sub-skill content, None for main SKILL.md.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_skill: Option<String>,

    /// Relative file path.
    pub file: String,

    /// Lowercase searchable content.
    pub content: String,

    /// Word count for TF-IDF calculations.
    pub word_count: usize,

    /// Extracted markdown headings.
    #[serde(default)]
    pub headings: Vec<String>,
}

impl ContentIndexEntry {
    /// Create a new content index entry.
    pub fn new(
        domain: String,
        sub_skill: Option<String>,
        file: String,
        content: String,
    ) -> Self {
        let word_count = content.split_whitespace().count();
        let headings = Self::extract_headings(&content);
        let content_lower = content.to_lowercase();

        Self {
            domain,
            sub_skill,
            file,
            content: content_lower,
            word_count,
            headings,
        }
    }

    /// Extract markdown headings from content.
    fn extract_headings(content: &str) -> Vec<String> {
        content
            .lines()
            .filter(|line| line.starts_with('#'))
            .map(|line| line.trim_start_matches('#').trim().to_string())
            .collect()
    }

    /// Check if this entry matches a search term.
    pub fn matches(&self, term: &str) -> bool {
        let term_lower = term.to_lowercase();
        self.content.contains(&term_lower)
    }

    /// Count occurrences of a term.
    pub fn count_matches(&self, term: &str) -> usize {
        let term_lower = term.to_lowercase();
        self.content.matches(&term_lower).count()
    }

    /// Generate a unique key for this entry.
    pub fn key(&self) -> String {
        match &self.sub_skill {
            Some(sub) => format!("{}:{}", self.domain, sub),
            None => self.domain.clone(),
        }
    }
}

/// Full content index mapping keys to entries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContentIndex {
    /// Map of unique keys to content entries.
    pub entries: HashMap<String, ContentIndexEntry>,

    /// ISO timestamp of last index update.
    pub last_updated: DateTime<Utc>,
}

impl ContentIndex {
    /// Create a new empty content index.
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            last_updated: Utc::now(),
        }
    }

    /// Add an entry to the index.
    pub fn insert(&mut self, entry: ContentIndexEntry) {
        let key = entry.key();
        self.entries.insert(key, entry);
        self.last_updated = Utc::now();
    }

    /// Get an entry by key.
    pub fn get(&self, key: &str) -> Option<&ContentIndexEntry> {
        self.entries.get(key)
    }

    /// Get entries for a specific domain.
    pub fn get_domain_entries(&self, domain: &str) -> Vec<&ContentIndexEntry> {
        self.entries
            .values()
            .filter(|e| e.domain == domain)
            .collect()
    }

    /// Get total entry count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if index is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate over all entries.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &ContentIndexEntry)> {
        self.entries.iter()
    }
}

// ── Skill Content ───────────────────────────────────────────────────────

/// Full skill content response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillContent {
    /// Skill name/identifier.
    pub name: String,

    /// SKILL.md content.
    pub content: String,

    /// Available sub-skill names.
    #[serde(default)]
    pub sub_skills: Vec<String>,

    /// Whether this skill has a references directory.
    pub has_references: bool,
}

impl SkillContent {
    /// Create a new skill content response.
    pub fn new(name: String, content: String) -> Self {
        Self {
            name,
            content,
            sub_skills: Vec::new(),
            has_references: false,
        }
    }

    /// Set sub-skills.
    pub fn with_sub_skills(mut self, sub_skills: Vec<String>) -> Self {
        self.sub_skills = sub_skills;
        self
    }

    /// Set has_references.
    pub fn with_references(mut self, has_references: bool) -> Self {
        self.has_references = has_references;
        self
    }
}

/// Sub-skill content response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubSkillContent {
    /// Parent skill domain.
    pub domain: String,

    /// Sub-skill name.
    pub sub_skill: String,

    /// Sub-skill markdown content.
    pub content: String,
}

impl SubSkillContent {
    /// Create a new sub-skill content response.
    pub fn new(domain: String, sub_skill: String, content: String) -> Self {
        Self {
            domain,
            sub_skill,
            content,
        }
    }
}

// ── Search ──────────────────────────────────────────────────────────────

/// How a search result was matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchType {
    /// Matched skill name.
    Name,
    /// Matched description.
    Description,
    /// Matched tags.
    Tags,
    /// Matched trigger words.
    Triggers,
    /// Matched content body.
    Content,
}

impl MatchType {
    /// Get the weight multiplier for this match type.
    pub fn weight(&self) -> f64 {
        match self {
            MatchType::Name => 3.0,
            MatchType::Triggers => 2.5,
            MatchType::Tags => 2.0,
            MatchType::Description => 1.5,
            MatchType::Content => 1.0,
        }
    }
}

/// A single search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Skill domain name.
    pub domain: String,

    /// Sub-skill name if matched within a sub-skill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_skill: Option<String>,

    /// Relevance score (0.0 to 1.0+).
    pub score: f64,

    /// How the match was found.
    pub match_type: MatchType,

    /// Optional excerpt showing match context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,

    /// Optional file path for content matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

impl SearchResult {
    /// Create a new search result.
    pub fn new(domain: String, score: f64, match_type: MatchType) -> Self {
        Self {
            domain,
            sub_skill: None,
            score,
            match_type,
            snippet: None,
            file: None,
        }
    }

    /// Set sub-skill.
    pub fn with_sub_skill(mut self, sub_skill: String) -> Self {
        self.sub_skill = Some(sub_skill);
        self
    }

    /// Set snippet.
    pub fn with_snippet(mut self, snippet: String) -> Self {
        self.snippet = Some(snippet);
        self
    }

    /// Set file path.
    pub fn with_file(mut self, file: String) -> Self {
        self.file = Some(file);
        self
    }

    /// Get a display-friendly identifier.
    pub fn display_id(&self) -> String {
        match &self.sub_skill {
            Some(sub) => format!("{}:{}", self.domain, sub),
            None => self.domain.clone(),
        }
    }
}

impl PartialEq for SearchResult {
    fn eq(&self, other: &Self) -> bool {
        self.domain == other.domain && self.sub_skill == other.sub_skill
    }
}

impl Eq for SearchResult {}

impl PartialOrd for SearchResult {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SearchResult {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Sort by score descending.
        other
            .score
            .partial_cmp(&self.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    }
}

/// Search query options.
#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    /// Maximum number of results to return.
    pub limit: Option<usize>,

    /// Minimum score threshold.
    pub min_score: Option<f64>,

    /// Only search specific match types.
    pub match_types: Option<Vec<MatchType>>,

    /// Filter to specific domains.
    pub domains: Option<Vec<String>>,
}

impl SearchOptions {
    /// Create with a limit.
    pub fn with_limit(limit: usize) -> Self {
        Self {
            limit: Some(limit),
            ..Default::default()
        }
    }
}

/// Results from a search operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResults {
    /// Matched results, sorted by relevance.
    pub results: Vec<SearchResult>,

    /// Original query.
    pub query: String,

    /// Total matches before limit applied.
    pub total_matches: usize,

    /// Whether results were truncated.
    pub truncated: bool,
}

impl SearchResults {
    /// Create new search results.
    pub fn new(query: String, mut results: Vec<SearchResult>, limit: Option<usize>) -> Self {
        results.sort();

        let total_matches = results.len();
        let truncated = limit.map(|l| total_matches > l).unwrap_or(false);

        if let Some(limit) = limit {
            results.truncate(limit);
        }

        Self {
            results,
            query,
            total_matches,
            truncated,
        }
    }

    /// Check if any results were found.
    pub fn is_empty(&self) -> bool {
        self.results.is_empty()
    }

    /// Get result count.
    pub fn len(&self) -> usize {
        self.results.len()
    }

    /// Get the top result if any.
    pub fn top(&self) -> Option<&SearchResult> {
        self.results.first()
    }
}

// ── Validation ──────────────────────────────────────────────────────────

/// Validation result for skill checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Whether all checks passed.
    pub valid: bool,

    /// Critical errors that must be fixed.
    pub errors: Vec<String>,

    /// Non-critical warnings.
    pub warnings: Vec<String>,

    /// Number of skills checked.
    pub skills_checked: usize,
}

impl ValidationResult {
    /// Create a passing result.
    pub fn pass(skills_checked: usize) -> Self {
        Self {
            valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
            skills_checked,
        }
    }

    /// Add an error.
    pub fn add_error(&mut self, error: String) {
        self.errors.push(error);
        self.valid = false;
    }

    /// Add a warning.
    pub fn add_warning(&mut self, warning: String) {
        self.warnings.push(warning);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_minimal_meta() {
        let json = r#"{"name": "test-skill", "description": "A test skill"}"#;
        let meta: SkillMeta = serde_json::from_str(json).unwrap();
        assert_eq!(meta.name, "test-skill");
        assert!(meta.tags.is_empty());
        assert!(!meta.has_sub_skills());
    }

    #[test]
    fn test_deserialize_full_meta() {
        let json = r#"{
            "name": "forms",
            "description": "Form handling patterns",
            "tags": ["validation", "input"],
            "sub_skills": [
                {"name": "react", "file": "react/SKILL.md", "triggers": ["useForm"]},
                {"name": "validation", "file": "validation/SKILL.md"}
            ],
            "source": "official"
        }"#;
        let meta: SkillMeta = serde_json::from_str(json).unwrap();
        assert!(meta.has_sub_skills());
        assert_eq!(meta.sub_skill_names(), vec!["react", "validation"]);
        assert_eq!(meta.find_sub_skill("react").unwrap().triggers, vec!["useForm"]);
    }

    #[test]
    fn test_all_triggers() {
        let meta = SkillMeta {
            name: "forms".to_string(),
            description: "Form handling".to_string(),
            tags: vec!["forms".to_string(), "input".to_string()],
            sub_skills: Some(vec![SubSkillMeta {
                name: "react".to_string(),
                file: "react/SKILL.md".to_string(),
                triggers: vec!["useForm".to_string()],
            }]),
            source: None,
        };
        let triggers = meta.all_triggers();
        assert!(triggers.contains(&"forms"));
        assert!(triggers.contains(&"useForm"));
    }

    #[test]
    fn test_skill_index_operations() {
        let meta = SkillMeta {
            name: "test".to_string(),
            description: "Test skill".to_string(),
            tags: vec![],
            sub_skills: None,
            source: None,
        };
        let index = SkillIndex::with_skills(vec![meta], vec![]);
        assert_eq!(index.len(), 1);
        assert!(!index.has_errors());
        assert!(index.find("test").is_some());
        assert!(index.find("nonexistent").is_none());
    }

    #[test]
    fn test_content_index_entry() {
        let entry = ContentIndexEntry::new(
            "forms".to_string(),
            Some("react".to_string()),
            "react/SKILL.md".to_string(),
            "# React Forms\n\nUse `useForm` hook.".to_string(),
        );
        assert_eq!(entry.key(), "forms:react");
        assert!(entry.matches("useForm"));
        assert!(entry.matches("USEFORM")); // case insensitive
        assert!(!entry.matches("angular"));
        assert_eq!(entry.headings, vec!["React Forms"]);
    }

    #[test]
    fn test_content_index() {
        let mut index = ContentIndex::new();
        index.insert(ContentIndexEntry::new(
            "forms".to_string(),
            None,
            "SKILL.md".to_string(),
            "Form handling patterns".to_string(),
        ));
        index.insert(ContentIndexEntry::new(
            "forms".to_string(),
            Some("react".to_string()),
            "react/SKILL.md".to_string(),
            "React form patterns".to_string(),
        ));
        assert_eq!(index.len(), 2);
        assert!(index.get("forms").is_some());
        assert!(index.get("forms:react").is_some());
        assert_eq!(index.get_domain_entries("forms").len(), 2);
    }

    #[test]
    fn test_match_type_weights() {
        assert!(MatchType::Name.weight() > MatchType::Content.weight());
        assert!(MatchType::Triggers.weight() > MatchType::Tags.weight());
    }

    #[test]
    fn test_search_result_ordering() {
        let mut results = [
            SearchResult::new("low".to_string(), 0.3, MatchType::Content),
            SearchResult::new("high".to_string(), 0.9, MatchType::Name),
            SearchResult::new("mid".to_string(), 0.6, MatchType::Tags),
        ];
        results.sort();
        assert_eq!(results[0].domain, "high");
        assert_eq!(results[1].domain, "mid");
        assert_eq!(results[2].domain, "low");
    }

    #[test]
    fn test_search_results_truncation() {
        let results = vec![
            SearchResult::new("a".to_string(), 0.9, MatchType::Name),
            SearchResult::new("b".to_string(), 0.8, MatchType::Name),
            SearchResult::new("c".to_string(), 0.7, MatchType::Name),
        ];
        let sr = SearchResults::new("test".to_string(), results, Some(2));
        assert_eq!(sr.len(), 2);
        assert_eq!(sr.total_matches, 3);
        assert!(sr.truncated);
    }

    #[test]
    fn test_skill_content_builder() {
        let content = SkillContent::new("forms".to_string(), "# Forms".to_string())
            .with_sub_skills(vec!["react".to_string()])
            .with_references(true);
        assert_eq!(content.sub_skills.len(), 1);
        assert!(content.has_references);
    }

    #[test]
    fn test_validation_result() {
        let mut result = ValidationResult::pass(10);
        assert!(result.valid);

        result.add_error("Missing _meta.json".to_string());
        assert!(!result.valid);
        assert_eq!(result.errors.len(), 1);

        result.add_warning("No tags defined".to_string());
        assert_eq!(result.warnings.len(), 1);
    }
}
