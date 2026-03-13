//! Command parsing and tool detection.
//!
//! Parses shell command strings into structured [`ParsedCommand`] values,
//! identifying the program, subcommand, arguments, and tool category.
//! Ported from ShellVault's `session::parser`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Parsed command information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedCommand {
    pub program: String,
    pub subcommand: Option<String>,
    pub args: Vec<String>,
    pub is_pipeline: bool,
    pub tool_category: ToolCategory,
}

/// Category of development tool.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolCategory {
    VersionControl,
    PackageManager,
    BuildTool,
    TestRunner,
    Linter,
    Shell,
    Editor,
    Container,
    Cloud,
    Database,
    #[default]
    Other,
}

/// Parser for extracting structured command information from raw shell strings.
pub struct CommandParser {
    tool_categories: HashMap<&'static str, ToolCategory>,
}

impl CommandParser {
    /// Create a new parser with built-in tool category mappings.
    pub fn new() -> Self {
        let mut categories = HashMap::new();

        // Version control
        for tool in ["git", "svn", "hg", "mercurial"] {
            categories.insert(tool, ToolCategory::VersionControl);
        }

        // Package managers
        for tool in [
            "npm", "yarn", "pnpm", "pip", "pip3", "poetry", "cargo", "go", "gem", "bundle",
            "composer", "nuget", "dotnet", "maven", "mvn", "gradle", "apt", "apt-get", "brew",
            "choco", "winget", "scoop",
        ] {
            categories.insert(tool, ToolCategory::PackageManager);
        }

        // Build tools
        for tool in [
            "make", "cmake", "ninja", "msbuild", "ant", "webpack", "vite", "esbuild", "rollup",
            "parcel", "tsc", "rustc", "gcc", "clang", "g++",
        ] {
            categories.insert(tool, ToolCategory::BuildTool);
        }

        // Test runners
        for tool in [
            "pytest",
            "jest",
            "mocha",
            "vitest",
            "cargo-test",
            "go-test",
            "rspec",
            "phpunit",
            "dotnet-test",
            "gradle-test",
            "mvn-test",
        ] {
            categories.insert(tool, ToolCategory::TestRunner);
        }

        // Linters
        for tool in [
            "eslint",
            "prettier",
            "black",
            "ruff",
            "flake8",
            "pylint",
            "mypy",
            "clippy",
            "rustfmt",
            "gofmt",
            "rubocop",
            "phpcs",
            "shellcheck",
        ] {
            categories.insert(tool, ToolCategory::Linter);
        }

        // Shell utilities
        for tool in [
            "cd", "ls", "dir", "pwd", "echo", "cat", "less", "more", "head", "tail", "grep",
            "find", "sed", "awk", "sort", "uniq", "wc", "cp", "mv", "rm", "mkdir", "rmdir",
            "touch", "chmod", "chown",
        ] {
            categories.insert(tool, ToolCategory::Shell);
        }

        // Editors
        for tool in ["vim", "nvim", "nano", "emacs", "code", "subl", "atom"] {
            categories.insert(tool, ToolCategory::Editor);
        }

        // Containers
        for tool in [
            "docker",
            "docker-compose",
            "podman",
            "kubectl",
            "k9s",
            "helm",
            "minikube",
            "kind",
        ] {
            categories.insert(tool, ToolCategory::Container);
        }

        // Cloud
        for tool in ["aws", "az", "gcloud", "terraform", "pulumi", "ansible"] {
            categories.insert(tool, ToolCategory::Cloud);
        }

        // Database
        for tool in ["psql", "mysql", "sqlite3", "mongo", "redis-cli", "sqlcmd"] {
            categories.insert(tool, ToolCategory::Database);
        }

        Self {
            tool_categories: categories,
        }
    }

    /// Parse a command string into structured information.
    ///
    /// Returns `None` for empty/whitespace-only input.
    pub fn parse(&self, command: &str) -> Option<ParsedCommand> {
        let command = command.trim();
        if command.is_empty() {
            return None;
        }

        // Check for pipeline.
        let is_pipeline = command.contains('|');

        // Get first command in pipeline.
        let first_cmd = command.split('|').next()?.trim();

        // Parse into parts.
        let parts: Vec<&str> = first_cmd.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }

        let program = parts[0].to_string();
        let program_base = program.split('/').next_back().unwrap_or(&program);
        let program_base = program_base.split('\\').next_back().unwrap_or(program_base);

        // Determine tool category.
        let category = self
            .tool_categories
            .get(program_base)
            .cloned()
            .unwrap_or(ToolCategory::Other);

        // Extract subcommand for known tools.
        let subcommand = self.extract_subcommand(program_base, &parts);

        // Extract arguments (skip program and subcommand).
        let args_start = if subcommand.is_some() { 2 } else { 1 };
        let args: Vec<String> = parts.iter().skip(args_start).map(|s| s.to_string()).collect();

        Some(ParsedCommand {
            program,
            subcommand,
            args,
            is_pipeline,
            tool_category: category,
        })
    }

    /// Extract subcommand for tools that use them.
    fn extract_subcommand(&self, program: &str, parts: &[&str]) -> Option<String> {
        if parts.len() < 2 {
            return None;
        }

        let potential_subcommand = parts[1];

        // Skip if it looks like a flag.
        if potential_subcommand.starts_with('-') {
            return None;
        }

        // Tools known to have subcommands.
        match program {
            "git" | "docker" | "docker-compose" | "kubectl" | "cargo" | "npm" | "yarn"
            | "pnpm" | "pip" | "pip3" | "poetry" | "go" | "dotnet" | "aws" | "az" | "gcloud"
            | "terraform" | "helm" | "bundle" => Some(potential_subcommand.to_string()),
            _ => None,
        }
    }

    /// Check if a command indicates testing activity.
    pub fn is_test_command(&self, command: &str) -> bool {
        let cmd_lower = command.to_lowercase();
        cmd_lower.contains("test")
            || cmd_lower.contains("pytest")
            || cmd_lower.contains("jest")
            || cmd_lower.contains("mocha")
            || cmd_lower.contains("vitest")
            || cmd_lower.contains("rspec")
    }

    /// Check if a command indicates build activity.
    pub fn is_build_command(&self, command: &str) -> bool {
        let cmd_lower = command.to_lowercase();
        cmd_lower.contains("build")
            || cmd_lower.contains("compile")
            || cmd_lower.contains("make")
            || cmd_lower.starts_with("cargo b")
            || cmd_lower.starts_with("npm run build")
            || cmd_lower.starts_with("yarn build")
    }
}

impl Default for CommandParser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_git_command() {
        let parser = CommandParser::new();
        let parsed = parser.parse("git commit -m 'test'").unwrap();

        assert_eq!(parsed.program, "git");
        assert_eq!(parsed.subcommand, Some("commit".to_string()));
        assert_eq!(parsed.tool_category, ToolCategory::VersionControl);
        assert!(!parsed.is_pipeline);
    }

    #[test]
    fn test_parse_pipeline() {
        let parser = CommandParser::new();
        let parsed = parser.parse("cat file.txt | grep pattern").unwrap();

        assert_eq!(parsed.program, "cat");
        assert!(parsed.is_pipeline);
        assert_eq!(parsed.tool_category, ToolCategory::Shell);
    }

    #[test]
    fn test_parse_cargo_test() {
        let parser = CommandParser::new();
        let parsed = parser.parse("cargo test --lib").unwrap();

        assert_eq!(parsed.program, "cargo");
        assert_eq!(parsed.subcommand, Some("test".to_string()));
        assert_eq!(parsed.tool_category, ToolCategory::PackageManager);
    }

    #[test]
    fn test_parse_empty_returns_none() {
        let parser = CommandParser::new();
        assert!(parser.parse("").is_none());
        assert!(parser.parse("   ").is_none());
    }

    #[test]
    fn test_parse_absolute_path_program() {
        let parser = CommandParser::new();
        let parsed = parser.parse("/usr/bin/git status").unwrap();

        assert_eq!(parsed.program, "/usr/bin/git");
        assert_eq!(parsed.subcommand, Some("status".to_string()));
        assert_eq!(parsed.tool_category, ToolCategory::VersionControl);
    }

    #[test]
    fn test_parse_docker_compose() {
        let parser = CommandParser::new();
        let parsed = parser.parse("docker-compose up -d").unwrap();

        assert_eq!(parsed.program, "docker-compose");
        assert_eq!(parsed.subcommand, Some("up".to_string()));
        assert_eq!(parsed.tool_category, ToolCategory::Container);
        assert_eq!(parsed.args, vec!["-d"]);
    }

    #[test]
    fn test_parse_flag_as_first_arg_no_subcommand() {
        let parser = CommandParser::new();
        let parsed = parser.parse("git --version").unwrap();

        assert_eq!(parsed.program, "git");
        assert!(parsed.subcommand.is_none());
    }

    #[test]
    fn test_is_test_command() {
        let parser = CommandParser::new();
        assert!(parser.is_test_command("cargo test --lib"));
        assert!(parser.is_test_command("pytest -v"));
        assert!(parser.is_test_command("npm run test"));
        assert!(!parser.is_test_command("cargo build"));
    }

    #[test]
    fn test_is_build_command() {
        let parser = CommandParser::new();
        assert!(parser.is_build_command("cargo build"));
        assert!(parser.is_build_command("make all"));
        assert!(parser.is_build_command("npm run build"));
        assert!(!parser.is_build_command("cargo test"));
    }

    #[test]
    fn test_parse_cloud_tool() {
        let parser = CommandParser::new();
        let parsed = parser.parse("aws s3 cp file.txt s3://bucket/").unwrap();

        assert_eq!(parsed.program, "aws");
        assert_eq!(parsed.subcommand, Some("s3".to_string()));
        assert_eq!(parsed.tool_category, ToolCategory::Cloud);
    }

    #[test]
    fn test_parse_database_tool() {
        let parser = CommandParser::new();
        let parsed = parser.parse("psql -U admin mydb").unwrap();

        assert_eq!(parsed.program, "psql");
        assert!(parsed.subcommand.is_none());
        assert_eq!(parsed.tool_category, ToolCategory::Database);
    }

    #[test]
    fn test_default_category_for_unknown() {
        let parser = CommandParser::new();
        let parsed = parser.parse("my-custom-tool --flag").unwrap();

        assert_eq!(parsed.tool_category, ToolCategory::Other);
    }
}
