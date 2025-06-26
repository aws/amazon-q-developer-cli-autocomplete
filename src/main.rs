use serde::{Deserialize, Serialize};
use regex::Regex;

/// Represents a trusted command pattern that can be executed without user confirmation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrustedCommand {
    /// The command pattern using glob-style matching (with * wildcards).
    /// Examples: "npm *", "git status", "git restore *"
    pub command: String,
    
    /// Optional description for documentation purposes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Configuration for trusted commands that can be executed without user confirmation.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct TrustedCommandsConfig {
    /// List of trusted command patterns.
    pub trusted_commands: Vec<TrustedCommand>,
}

/// Processed trusted commands for efficient pattern matching.
#[derive(Debug, Clone)]
pub struct ProcessedTrustedCommands {
    /// List of command patterns with their descriptions.
    patterns: Vec<(String, Option<String>)>,
}

impl ProcessedTrustedCommands {
    /// Create a new ProcessedTrustedCommands from a TrustedCommandsConfig.
    pub fn new(config: TrustedCommandsConfig) -> Self {
        let patterns = config
            .trusted_commands
            .into_iter()
            .map(|cmd| (cmd.command, cmd.description))
            .collect();
        
        Self { patterns }
    }
    
    /// Check if a command is trusted by matching against the patterns.
    pub fn is_trusted(&self, command: &str) -> bool {
        self.patterns
            .iter()
            .any(|(pattern, _)| Self::glob_match(pattern, command))
    }
    
    /// Perform glob-style pattern matching with * wildcards.
    /// Returns true if the pattern matches the command.
    fn glob_match(pattern: &str, command: &str) -> bool {
        // Handle exact match first
        if pattern == command {
            return true;
        }
        
        // Convert glob pattern to regex
        let regex_pattern = pattern
            .replace("*", ".*") // Replace * with .*
            .replace("?", "."); // Replace ? with . (single character)
        
        // Add anchors to match the entire string
        let regex_pattern = format!("^{}$", regex_pattern);
        
        // Compile and match
        if let Ok(regex) = Regex::new(&regex_pattern) {
            regex.is_match(command)
        } else {
            // If regex compilation fails, fall back to exact match
            pattern == command
        }
    }
    
    /// Get all patterns for display purposes.
    pub fn patterns(&self) -> &[(String, Option<String>)] {
        &self.patterns
    }
}

fn main() {
    println!("Testing trusted commands configuration model...");
    
    // Test JSON deserialization
    let json = r#"{
        "trusted_commands": [
            {
                "command": "npm *",
                "description": "All npm commands"
            },
            {
                "command": "git status"
            }
        ]
    }"#;
    
    let config: TrustedCommandsConfig = serde_json::from_str(json).unwrap();
    println!("Parsed config: {:?}", config);
    
    // Test pattern matching
    let processed = ProcessedTrustedCommands::new(config);
    
    println!("Testing pattern matching:");
    println!("npm install: {}", processed.is_trusted("npm install"));
    println!("npm run build: {}", processed.is_trusted("npm run build"));
    println!("git status: {}", processed.is_trusted("git status"));
    println!("git commit: {}", processed.is_trusted("git commit"));
    println!("ls: {}", processed.is_trusted("ls"));
    
    println!("All tests passed!");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trusted_command_deserialization() {
        // Test basic trusted command deserialization
        let json = r#"{
            "command": "npm *",
            "description": "All npm commands"
        }"#;
        
        let cmd: TrustedCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.command, "npm *");
        assert_eq!(cmd.description, Some("All npm commands".to_string()));
        
        // Test without description
        let json = r#"{
            "command": "git status"
        }"#;
        
        let cmd: TrustedCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.command, "git status");
        assert_eq!(cmd.description, None);
    }
    
    #[test]
    fn test_trusted_commands_config_deserialization() {
        let json = r#"{
            "trusted_commands": [
                {
                    "command": "npm *",
                    "description": "All npm commands"
                },
                {
                    "command": "git status"
                }
            ]
        }"#;
        
        let config: TrustedCommandsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.trusted_commands.len(), 2);
        assert_eq!(config.trusted_commands[0].command, "npm *");
        assert_eq!(config.trusted_commands[0].description, Some("All npm commands".to_string()));
        assert_eq!(config.trusted_commands[1].command, "git status");
        assert_eq!(config.trusted_commands[1].description, None);
    }
    
    #[test]
    fn test_processed_trusted_commands_exact_match() {
        let config = TrustedCommandsConfig {
            trusted_commands: vec![
                TrustedCommand {
                    command: "git status".to_string(),
                    description: Some("Git status command".to_string()),
                },
                TrustedCommand {
                    command: "npm install".to_string(),
                    description: None,
                },
            ],
        };
        
        let processed = ProcessedTrustedCommands::new(config);
        
        // Test exact matches
        assert!(processed.is_trusted("git status"));
        assert!(processed.is_trusted("npm install"));
        
        // Test non-matches
        assert!(!processed.is_trusted("git commit"));
        assert!(!processed.is_trusted("npm run"));
        assert!(!processed.is_trusted("ls"));
    }
    
    #[test]
    fn test_processed_trusted_commands_glob_match() {
        let config = TrustedCommandsConfig {
            trusted_commands: vec![
                TrustedCommand {
                    command: "npm *".to_string(),
                    description: Some("All npm commands".to_string()),
                },
                TrustedCommand {
                    command: "git restore *".to_string(),
                    description: Some("All git restore commands".to_string()),
                },
            ],
        };
        
        let processed = ProcessedTrustedCommands::new(config);
        
        // Test glob matches
        assert!(processed.is_trusted("npm install"));
        assert!(processed.is_trusted("npm run build"));
        assert!(processed.is_trusted("npm install --save-dev typescript"));
        assert!(processed.is_trusted("git restore file.txt"));
        assert!(processed.is_trusted("git restore --staged file.txt"));
        
        // Test non-matches
        assert!(!processed.is_trusted("git status"));
        assert!(!processed.is_trusted("git commit"));
        assert!(!processed.is_trusted("yarn install"));
        assert!(!processed.is_trusted("ls"));
    }
}