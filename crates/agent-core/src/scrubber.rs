//! Secret scrubbing for safe export and display.
//!
//! Detects and redacts secrets (API keys, tokens, passwords, private keys)
//! from text using configurable regex patterns. Ported from ShellVault.

use regex::Regex;
use std::borrow::Cow;

/// Default patterns for secret detection.
const DEFAULT_PATTERNS: &[&str] = &[
    // API keys and tokens
    r#"(?i)(api[_-]?key|token|secret|password|passwd|pwd)\s*[=:]\s*['"]?[^\s'""]+"#,
    // Bearer tokens
    r"(?i)bearer\s+\S+",
    // AWS keys
    r"(?i)aws[_-]?(access[_-]?key[_-]?id|secret[_-]?access[_-]?key)\s*[=:]\s*\S+",
    // Generic secrets in environment variables
    r"(?i)(export\s+)?[A-Z_]*(?:SECRET|TOKEN|KEY|PASSWORD|CREDENTIAL)[A-Z_]*\s*=\s*\S+",
    // GitHub tokens
    r"ghp_[a-zA-Z0-9]{36}",
    r"gho_[a-zA-Z0-9]{36}",
    r"ghs_[a-zA-Z0-9]{36}",
    // Private keys
    r"-----BEGIN\s+(?:RSA\s+)?PRIVATE\s+KEY-----",
    // Base64-encoded secrets (long strings that look like tokens)
    r"(?i)(?:key|token|secret|password)\s*[=:]\s*[A-Za-z0-9+/]{32,}={0,2}",
];

/// Scrubber for removing secrets from text.
pub struct SecretScrubber {
    patterns: Vec<Regex>,
    replacement: String,
}

impl SecretScrubber {
    /// Create a new scrubber with default patterns.
    pub fn new() -> Self {
        Self::with_patterns(DEFAULT_PATTERNS.iter().map(|s| s.to_string()).collect())
    }

    /// Create a scrubber with custom patterns.
    pub fn with_patterns(patterns: Vec<String>) -> Self {
        let compiled: Vec<Regex> = patterns.iter().filter_map(|p| Regex::new(p).ok()).collect();

        Self {
            patterns: compiled,
            replacement: "[REDACTED]".to_string(),
        }
    }

    /// Set the replacement string.
    pub fn with_replacement(mut self, replacement: impl Into<String>) -> Self {
        self.replacement = replacement.into();
        self
    }

    /// Add an additional pattern.
    pub fn add_pattern(&mut self, pattern: &str) -> Result<(), regex::Error> {
        let regex = Regex::new(pattern)?;
        self.patterns.push(regex);
        Ok(())
    }

    /// Scrub secrets from text.
    pub fn scrub<'a>(&self, text: &'a str) -> Cow<'a, str> {
        let mut result = Cow::Borrowed(text);

        for pattern in &self.patterns {
            if pattern.is_match(&result) {
                result = Cow::Owned(pattern.replace_all(&result, &*self.replacement).to_string());
            }
        }

        result
    }

    /// Scrub secrets from a command string, preserving structure.
    pub fn scrub_command(&self, command: &str) -> String {
        self.scrub(command).to_string()
    }

    /// Scrub secrets from multiple lines.
    pub fn scrub_lines(&self, lines: &[String]) -> Vec<String> {
        lines.iter().map(|l| self.scrub(l).to_string()).collect()
    }

    /// Check if text contains potential secrets.
    pub fn has_secrets(&self, text: &str) -> bool {
        self.patterns.iter().any(|p| p.is_match(text))
    }
}

impl Default for SecretScrubber {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scrub_api_key() {
        let scrubber = SecretScrubber::new();

        let input = "curl -H 'Authorization: Bearer abc123xyz'";
        let result = scrubber.scrub(input);
        assert!(result.contains("[REDACTED]"));
        assert!(!result.contains("abc123xyz"));
    }

    #[test]
    fn test_scrub_env_var() {
        let scrubber = SecretScrubber::new();

        let input = "export API_KEY=supersecret123";
        let result = scrubber.scrub(input);
        assert!(result.contains("[REDACTED]"));
        assert!(!result.contains("supersecret123"));
    }

    #[test]
    fn test_scrub_password() {
        let scrubber = SecretScrubber::new();

        let input = "mysql -u root -p password=secret123";
        let result = scrubber.scrub(input);
        assert!(result.contains("[REDACTED]"));
    }

    #[test]
    fn test_no_secrets() {
        let scrubber = SecretScrubber::new();

        let input = "git commit -m 'Add new feature'";
        let result = scrubber.scrub(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_github_token() {
        let scrubber = SecretScrubber::new();

        let input =
            "git clone https://ghp_abcdefghijklmnopqrstuvwxyz1234567890@github.com/user/repo";
        let result = scrubber.scrub(input);
        assert!(result.contains("[REDACTED]"));
    }

    #[test]
    fn test_custom_replacement() {
        let scrubber = SecretScrubber::new().with_replacement("***");

        let input = "export SECRET_KEY=mysecret";
        let result = scrubber.scrub(input);
        assert!(result.contains("***"));
        assert!(!result.contains("[REDACTED]"));
    }

    #[test]
    fn test_has_secrets() {
        let scrubber = SecretScrubber::new();

        assert!(scrubber.has_secrets("export API_KEY=abc123"));
        assert!(!scrubber.has_secrets("ls -la"));
    }

    #[test]
    fn test_scrub_lines() {
        let scrubber = SecretScrubber::new();
        let lines = vec![
            "echo hello".to_string(),
            "export TOKEN=secret123".to_string(),
            "ls -la".to_string(),
        ];
        let result = scrubber.scrub_lines(&lines);
        assert_eq!(result[0], "echo hello");
        assert!(result[1].contains("[REDACTED]"));
        assert_eq!(result[2], "ls -la");
    }

    #[test]
    fn test_private_key_detection() {
        let scrubber = SecretScrubber::new();

        let input = "-----BEGIN RSA PRIVATE KEY-----";
        assert!(scrubber.has_secrets(input));
        let result = scrubber.scrub(input);
        assert!(result.contains("[REDACTED]"));
    }

    #[test]
    fn test_add_custom_pattern() {
        let mut scrubber = SecretScrubber::new();
        scrubber.add_pattern(r"my-custom-secret-\d+").unwrap();

        let input = "found my-custom-secret-42 in logs";
        assert!(scrubber.has_secrets(input));
        let result = scrubber.scrub(input);
        assert!(result.contains("[REDACTED]"));
    }
}
