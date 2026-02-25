//! Skill loading tool — lets the agent search and load skills.

use agent_core::error::AgentError;
use agent_core::tool_registry::Tool;
use agent_skills::{SearchOptions, SearchService, SkillIndexer};
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// Tool that allows the agent to search for and load skills.
pub struct SkillLoadTool {
    indexer: Arc<SkillIndexer>,
    search: SearchService,
}

impl SkillLoadTool {
    /// Create a new skill_load tool.
    pub fn new(indexer: Arc<SkillIndexer>) -> Self {
        let search = SearchService::new(indexer.clone());
        Self { indexer, search }
    }

    fn err(msg: impl Into<String>) -> AgentError {
        AgentError::ToolExecution {
            tool_name: "skill_load".into(),
            message: msg.into(),
        }
    }
}

#[async_trait]
impl Tool for SkillLoadTool {
    fn name(&self) -> &str {
        "skill_load"
    }

    fn description(&self) -> &str {
        "Search for and load skill documents. Use 'search' action to find skills by query, or 'load' action to retrieve a specific skill's content."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["search", "load", "list"],
                    "description": "Action to perform: 'search' to find skills, 'load' to get skill content, 'list' to list all skills"
                },
                "query": {
                    "type": "string",
                    "description": "Search query (required for 'search' action)"
                },
                "skill": {
                    "type": "string",
                    "description": "Skill name to load (required for 'load' action)"
                },
                "sub_skill": {
                    "type": "string",
                    "description": "Optional sub-skill name when loading"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of search results (default: 5)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value) -> Result<String, AgentError> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Self::err("Missing 'action' parameter"))?;

        match action {
            "search" => {
                let query = args
                    .get("query")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| Self::err("Missing 'query' for search action"))?;

                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize)
                    .unwrap_or(5);

                let options = SearchOptions::with_limit(limit);
                let results = self.search.search_all(query, &options);

                if results.is_empty() {
                    return Ok(format!("No skills found matching '{}'.", query));
                }

                let mut output =
                    format!("Found {} skill(s) matching '{}':\n\n", results.len(), query);

                for result in &results.results {
                    output.push_str(&format!(
                        "- **{}** (score: {:.2}, matched: {:?})",
                        result.display_id(),
                        result.score,
                        result.match_type
                    ));
                    if let Some(snippet) = &result.snippet {
                        output.push_str(&format!("\n  > {}", snippet));
                    }
                    output.push('\n');
                }

                if results.truncated {
                    output.push_str(&format!(
                        "\n({} total matches, showing top {})",
                        results.total_matches,
                        results.len()
                    ));
                }

                Ok(output)
            }
            "load" => {
                let skill_name = args
                    .get("skill")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| Self::err("Missing 'skill' for load action"))?;

                let sub_skill = args.get("sub_skill").and_then(|v| v.as_str());

                if let Some(sub) = sub_skill {
                    let content = self
                        .indexer
                        .read_sub_skill_content(skill_name, sub)
                        .map_err(|e| Self::err(e.to_string()))?;

                    Ok(format!(
                        "# {}:{}\n\n{}",
                        content.domain, content.sub_skill, content.content
                    ))
                } else {
                    let content = self
                        .indexer
                        .read_skill_content(skill_name)
                        .map_err(|e| Self::err(e.to_string()))?;

                    let mut output = content.content.clone();

                    if !content.sub_skills.is_empty() {
                        output.push_str("\n\n---\n**Available sub-skills:** ");
                        output.push_str(&content.sub_skills.join(", "));
                        output.push_str(
                            "\n\nUse `skill_load` with `sub_skill` parameter to load a specific sub-skill.",
                        );
                    }

                    Ok(output)
                }
            }
            "list" => {
                let index = self.indexer.get_skill_index();

                if index.is_empty() {
                    return Ok("No skills loaded.".to_string());
                }

                let mut output = format!("{} skill(s) available:\n\n", index.len());

                for skill in &index.skills {
                    output.push_str(&format!("- **{}**: {}", skill.name, skill.description));
                    if !skill.tags.is_empty() {
                        output.push_str(&format!(" [{}]", skill.tags.join(", ")));
                    }
                    if skill.has_sub_skills() {
                        output.push_str(&format!(
                            " (sub-skills: {})",
                            skill.sub_skill_names().join(", ")
                        ));
                    }
                    output.push('\n');
                }

                Ok(output)
            }
            _ => Err(Self::err(format!(
                "Unknown action '{}'. Use 'search', 'load', or 'list'.",
                action
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_indexer() -> (TempDir, Arc<SkillIndexer>) {
        let temp_dir = TempDir::new().unwrap();

        let skill_dir = temp_dir.path().join("forms");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("_meta.json"),
            r#"{"name": "forms", "description": "Form handling patterns", "tags": ["validation", "input"]}"#,
        )
        .unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "# Forms\n\nUse these patterns for form handling.",
        )
        .unwrap();

        let skill_dir2 = temp_dir.path().join("auth");
        fs::create_dir_all(&skill_dir2).unwrap();
        fs::write(
            skill_dir2.join("_meta.json"),
            r#"{"name": "auth", "description": "Authentication patterns", "tags": ["jwt", "oauth"]}"#,
        )
        .unwrap();
        fs::write(
            skill_dir2.join("SKILL.md"),
            "# Auth\n\nAuthentication and authorization patterns.",
        )
        .unwrap();

        let indexer = Arc::new(SkillIndexer::new(temp_dir.path()));
        indexer.reload().unwrap();

        (temp_dir, indexer)
    }

    #[tokio::test]
    async fn test_skill_load_list() {
        let (_tmp, indexer) = setup_test_indexer();
        let tool = SkillLoadTool::new(indexer);

        let result = tool
            .execute(serde_json::json!({"action": "list"}))
            .await
            .unwrap();

        assert!(result.contains("forms"));
        assert!(result.contains("auth"));
        assert!(result.contains("2 skill(s)"));
    }

    #[tokio::test]
    async fn test_skill_load_search() {
        let (_tmp, indexer) = setup_test_indexer();
        let tool = SkillLoadTool::new(indexer);

        let result = tool
            .execute(serde_json::json!({"action": "search", "query": "forms"}))
            .await
            .unwrap();

        assert!(result.contains("forms"));
    }

    #[tokio::test]
    async fn test_skill_load_load() {
        let (_tmp, indexer) = setup_test_indexer();
        let tool = SkillLoadTool::new(indexer);

        let result = tool
            .execute(serde_json::json!({"action": "load", "skill": "forms"}))
            .await
            .unwrap();

        assert!(result.contains("# Forms"));
        assert!(result.contains("form handling"));
    }

    #[tokio::test]
    async fn test_skill_load_missing() {
        let (_tmp, indexer) = setup_test_indexer();
        let tool = SkillLoadTool::new(indexer);

        let result = tool
            .execute(serde_json::json!({"action": "load", "skill": "nonexistent"}))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_skill_load_invalid_action() {
        let (_tmp, indexer) = setup_test_indexer();
        let tool = SkillLoadTool::new(indexer);

        let result = tool.execute(serde_json::json!({"action": "invalid"})).await;

        assert!(result.is_err());
    }
}
