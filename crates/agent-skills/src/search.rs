//! Search service and snippet extraction for skills.

use std::sync::Arc;

use tracing::debug;

use crate::indexer::SkillIndexer;
use crate::models::{MatchType, SearchOptions, SearchResult, SearchResults, SkillMeta};

// ── Snippet Extraction ──────────────────────────────────────────────────

/// Extract a snippet around a search term match.
///
/// Returns a portion of the content centered around the first match,
/// with ellipsis indicators if truncated.
pub fn extract_snippet(content: &str, term: &str, context_chars: usize) -> Option<String> {
    let content_lower = content.to_lowercase();
    let term_lower = term.to_lowercase();

    let pos = content_lower.find(&term_lower)?;

    let start = pos.saturating_sub(context_chars);
    let end = (pos + term.len() + context_chars).min(content.len());

    // Snap to word boundaries.
    let start = find_word_start(content, start);
    let end = find_word_end(content, end);

    let mut snippet = String::new();
    if start > 0 {
        snippet.push_str("...");
    }
    snippet.push_str(content[start..end].trim());
    if end < content.len() {
        snippet.push_str("...");
    }

    // Clean up whitespace.
    let snippet = snippet
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    Some(snippet)
}

/// Find the start of a word boundary.
fn find_word_start(content: &str, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let bytes = content.as_bytes();
    let mut start = pos;
    while start > 0 && !bytes[start - 1].is_ascii_whitespace() {
        start -= 1;
    }
    start
}

/// Find the end of a word boundary.
fn find_word_end(content: &str, pos: usize) -> usize {
    if pos >= content.len() {
        return content.len();
    }
    let bytes = content.as_bytes();
    let mut end = pos;
    while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
        end += 1;
    }
    end
}

// ── Search Service ──────────────────────────────────────────────────────

/// Search service for querying skills and content.
pub struct SearchService {
    indexer: Arc<SkillIndexer>,
}

impl SearchService {
    /// Default context size for snippets.
    const DEFAULT_SNIPPET_CONTEXT: usize = 50;

    /// Create a new search service.
    pub fn new(indexer: Arc<SkillIndexer>) -> Self {
        Self { indexer }
    }

    /// Search skills by metadata (name, description, tags, triggers).
    pub fn search_skills(&self, query: &str, options: &SearchOptions) -> SearchResults {
        let skill_index = self.indexer.get_skill_index();
        let query_lower = query.to_lowercase();
        let terms: Vec<&str> = query_lower.split_whitespace().collect();

        let mut results = Vec::new();

        for skill in &skill_index.skills {
            if let Some(result) = self.match_skill(skill, &query_lower, &terms) {
                if let Some(ref domains) = options.domains {
                    if !domains.contains(&skill.name) {
                        continue;
                    }
                }

                if let Some(ref match_types) = options.match_types {
                    if !match_types.contains(&result.match_type) {
                        continue;
                    }
                }

                if let Some(min_score) = options.min_score {
                    if result.score < min_score {
                        continue;
                    }
                }

                results.push(result);
            }
        }

        debug!("Skill search '{}' found {} results", query, results.len());

        SearchResults::new(query.to_string(), results, options.limit)
    }

    /// Search content by full-text matching.
    pub fn search_content(&self, query: &str, options: &SearchOptions) -> SearchResults {
        let content_index = self.indexer.get_content_index();
        let query_lower = query.to_lowercase();
        let terms: Vec<&str> = query_lower.split_whitespace().collect();

        let mut results = Vec::new();

        for (_, entry) in content_index.iter() {
            if let Some(ref domains) = options.domains {
                if !domains.contains(&entry.domain) {
                    continue;
                }
            }

            let match_count: usize = terms.iter().map(|t| entry.count_matches(t)).sum();

            if match_count == 0 {
                continue;
            }

            // TF-IDF-like scoring.
            let tf = match_count as f64 / entry.word_count.max(1) as f64;
            let score = tf * MatchType::Content.weight();

            if let Some(min_score) = options.min_score {
                if score < min_score {
                    continue;
                }
            }

            let snippet =
                extract_snippet(&entry.content, &query_lower, Self::DEFAULT_SNIPPET_CONTEXT);

            let mut result = SearchResult::new(entry.domain.clone(), score, MatchType::Content)
                .with_file(entry.file.clone());

            if let Some(sub) = &entry.sub_skill {
                result = result.with_sub_skill(sub.clone());
            }

            if let Some(snippet) = snippet {
                result = result.with_snippet(snippet);
            }

            results.push(result);
        }

        debug!("Content search '{}' found {} results", query, results.len());

        SearchResults::new(query.to_string(), results, options.limit)
    }

    /// Combined search across both skills and content.
    pub fn search_all(&self, query: &str, options: &SearchOptions) -> SearchResults {
        let skill_results = self.search_skills(query, options);
        let content_results = self.search_content(query, options);

        let mut all_results = skill_results.results;

        for content_result in content_results.results {
            let exists = all_results.iter().any(|r| {
                r.domain == content_result.domain && r.sub_skill == content_result.sub_skill
            });

            if !exists {
                all_results.push(content_result);
            }
        }

        SearchResults::new(query.to_string(), all_results, options.limit)
    }

    /// Match a skill against search terms.
    fn match_skill(&self, skill: &SkillMeta, query: &str, terms: &[&str]) -> Option<SearchResult> {
        let name_lower = skill.name.to_lowercase();
        let desc_lower = skill.description.to_lowercase();

        // Exact name match (highest priority).
        if name_lower == query {
            return Some(SearchResult::new(
                skill.name.clone(),
                1.0 * MatchType::Name.weight(),
                MatchType::Name,
            ));
        }

        // Name contains query.
        if name_lower.contains(query) {
            return Some(SearchResult::new(
                skill.name.clone(),
                0.8 * MatchType::Name.weight(),
                MatchType::Name,
            ));
        }

        // Check tags.
        let tags: Vec<String> = skill.tags.iter().map(|s| s.to_lowercase()).collect();
        for tag in &tags {
            if tag == query || tag.contains(query) {
                return Some(SearchResult::new(
                    skill.name.clone(),
                    0.9 * MatchType::Tags.weight(),
                    MatchType::Tags,
                ));
            }
        }

        // Check sub-skill triggers.
        if let Some(subs) = &skill.sub_skills {
            for sub in subs {
                for trigger in &sub.triggers {
                    let trigger_lower = trigger.to_lowercase();
                    if trigger_lower == query || trigger_lower.contains(query) {
                        return Some(SearchResult::new(
                            skill.name.clone(),
                            0.9 * MatchType::Triggers.weight(),
                            MatchType::Triggers,
                        ));
                    }
                }
            }
        }

        // Description match.
        let term_matches: usize = terms.iter().filter(|t| desc_lower.contains(*t)).count();

        if term_matches > 0 {
            let score =
                (term_matches as f64 / terms.len() as f64) * MatchType::Description.weight();
            return Some(
                SearchResult::new(skill.name.clone(), score, MatchType::Description)
                    .with_snippet(skill.description.clone()),
            );
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::SubSkillMeta;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_skill(dir: &std::path::Path, meta: &SkillMeta) {
        let skill_dir = dir.join(&meta.name);
        fs::create_dir_all(&skill_dir).unwrap();

        let meta_json = serde_json::to_string_pretty(meta).unwrap();
        fs::write(skill_dir.join("_meta.json"), meta_json).unwrap();

        let content = format!("# {}\n\n{}", meta.name, meta.description);
        fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    #[test]
    fn test_extract_snippet_basic() {
        let content = "This is a test of the snippet extraction function.";
        let snippet = extract_snippet(content, "snippet", 10).unwrap();
        assert!(snippet.contains("snippet"));
    }

    #[test]
    fn test_extract_snippet_at_start() {
        let content = "Test content here with more words";
        let snippet = extract_snippet(content, "Test", 10).unwrap();
        assert!(snippet.starts_with("Test"));
        assert!(snippet.ends_with("..."));
    }

    #[test]
    fn test_extract_snippet_not_found() {
        let content = "This content doesn't have the search term";
        let snippet = extract_snippet(content, "missing", 10);
        assert!(snippet.is_none());
    }

    #[test]
    fn test_extract_snippet_case_insensitive() {
        let content = "This has a TERM in it";
        let snippet = extract_snippet(content, "term", 10).unwrap();
        assert!(snippet.to_lowercase().contains("term"));
    }

    #[test]
    fn test_search_by_name() {
        let temp_dir = TempDir::new().unwrap();
        let meta = SkillMeta {
            name: "forms".to_string(),
            description: "Form handling patterns".to_string(),
            tags: vec!["validation".to_string()],
            sub_skills: None,
            source: None,
        };
        create_test_skill(temp_dir.path(), &meta);

        let indexer = Arc::new(SkillIndexer::new(temp_dir.path()));
        indexer.reload().unwrap();

        let service = SearchService::new(indexer);
        let results = service.search_skills("forms", &SearchOptions::default());

        assert!(!results.is_empty());
        assert_eq!(results.top().unwrap().domain, "forms");
        assert_eq!(results.top().unwrap().match_type, MatchType::Name);
    }

    #[test]
    fn test_search_by_tag() {
        let temp_dir = TempDir::new().unwrap();
        let meta = SkillMeta {
            name: "forms".to_string(),
            description: "Form handling patterns".to_string(),
            tags: vec!["schema-validation".to_string(), "input".to_string()],
            sub_skills: None,
            source: None,
        };
        create_test_skill(temp_dir.path(), &meta);

        let indexer = Arc::new(SkillIndexer::new(temp_dir.path()));
        indexer.reload().unwrap();

        let service = SearchService::new(indexer);
        let results = service.search_skills("schema-validation", &SearchOptions::default());

        assert!(!results.is_empty());
        assert_eq!(results.top().unwrap().match_type, MatchType::Tags);
    }

    #[test]
    fn test_search_by_trigger() {
        let temp_dir = TempDir::new().unwrap();
        let meta = SkillMeta {
            name: "forms".to_string(),
            description: "Form handling patterns".to_string(),
            tags: vec![],
            sub_skills: Some(vec![SubSkillMeta {
                name: "react".to_string(),
                file: "react/SKILL.md".to_string(),
                triggers: vec!["useForm".to_string(), "react-hook-form".to_string()],
            }]),
            source: None,
        };
        create_test_skill(temp_dir.path(), &meta);

        let indexer = Arc::new(SkillIndexer::new(temp_dir.path()));
        indexer.reload().unwrap();

        let service = SearchService::new(indexer);
        let results = service.search_skills("useForm", &SearchOptions::default());

        assert!(!results.is_empty());
        assert_eq!(results.top().unwrap().match_type, MatchType::Triggers);
    }

    #[test]
    fn test_search_no_results() {
        let temp_dir = TempDir::new().unwrap();
        let meta = SkillMeta {
            name: "forms".to_string(),
            description: "Form handling patterns".to_string(),
            tags: vec![],
            sub_skills: None,
            source: None,
        };
        create_test_skill(temp_dir.path(), &meta);

        let indexer = Arc::new(SkillIndexer::new(temp_dir.path()));
        indexer.reload().unwrap();

        let service = SearchService::new(indexer);
        let results = service.search_skills("nonexistent", &SearchOptions::default());

        assert!(results.is_empty());
    }

    #[test]
    fn test_search_all_deduplicates() {
        let temp_dir = TempDir::new().unwrap();
        let meta = SkillMeta {
            name: "forms".to_string(),
            description: "Form handling patterns".to_string(),
            tags: vec!["forms".to_string()],
            sub_skills: None,
            source: None,
        };
        create_test_skill(temp_dir.path(), &meta);

        let indexer = Arc::new(SkillIndexer::new(temp_dir.path()));
        indexer.reload().unwrap();

        let service = SearchService::new(indexer);
        let results = service.search_all("forms", &SearchOptions::default());

        // "forms" matches both name (skill search) and content.
        // Deduplication should keep only one result per domain.
        let forms_results: Vec<_> = results
            .results
            .iter()
            .filter(|r| r.domain == "forms")
            .collect();
        assert_eq!(forms_results.len(), 1);
    }

    #[test]
    fn test_content_search() {
        let temp_dir = TempDir::new().unwrap();
        let meta = SkillMeta {
            name: "testing".to_string(),
            description: "Testing patterns".to_string(),
            tags: vec![],
            sub_skills: None,
            source: None,
        };
        let skill_dir = temp_dir.path().join("testing");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let meta_json = serde_json::to_string(&meta).unwrap();
        std::fs::write(skill_dir.join("_meta.json"), meta_json).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "# Testing\n\nUse pytest for unit tests and integration tests.",
        )
        .unwrap();

        let indexer = Arc::new(SkillIndexer::new(temp_dir.path()));
        indexer.reload().unwrap();

        let service = SearchService::new(indexer);
        let results = service.search_content("pytest", &SearchOptions::default());

        assert!(!results.is_empty());
        assert_eq!(results.top().unwrap().domain, "testing");
        assert_eq!(results.top().unwrap().match_type, MatchType::Content);
    }
}
