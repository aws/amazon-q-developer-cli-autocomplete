use std::collections::HashMap;
use std::io::Write;

use crossterm::queue;
use crossterm::style::{
    self,
    Color,
};
use eyre::Result;
use serde::{
    Deserialize,
    Serialize,
};

use crate::cli::chat::tools::{
    InvokeOutput,
    OutputKind,
    ToolSpec,
};
use crate::cli::chat::{
    CONTINUATION_LINE,
    PURPOSE_ARROW,
};

/// Schema for the internal_command tool
///
/// This tool allows the AI to suggest commands within the Q chat system
/// when a user's natural language query indicates they want to perform a specific action.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct InternalCommand {
    /// The command to execute (without the leading slash)
    pub command: String,
    /// Optional subcommand for commands that support them
    pub subcommand: Option<String>,
    /// Optional arguments for the command
    pub args: Option<Vec<String>>,
    /// Optional flags for the command
    pub flags: Option<HashMap<String, String>>,
    /// Optional summary description for the command
    pub summary: Option<String>,
}

impl InternalCommand {
    /// Check if the command requires user acceptance
    /// Uses a "safe list" approach - only explicitly safe commands are allowed without confirmation
    pub fn requires_acceptance(&self) -> bool {
        let command_string = self.build_command_string();

        // Safe/read-only commands that don't require user acceptance
        let safe_commands = [
            "/help",
            "/usage",
            "/context show",
            "/profile list",
            "/tools schema",
            "/prompts list",
            "/model",
        ];

        // Check if this is a safe command
        let is_safe = safe_commands
            .iter()
            .any(|safe_cmd| command_string.starts_with(safe_cmd));

        // Require acceptance for all commands except the explicitly safe ones
        !is_safe
    }

    /// Build the command string for display and processing
    pub fn build_command_string(&self) -> String {
        let mut command_parts = vec![format!("/{}", self.command)];

        if let Some(ref subcommand) = self.subcommand {
            command_parts.push(subcommand.clone());
        }

        if let Some(ref args) = self.args {
            command_parts.extend(args.clone());
        }

        if let Some(ref flags) = self.flags {
            for (key, value) in flags {
                if value.is_empty() {
                    command_parts.push(format!("--{}", key));
                } else {
                    command_parts.push(format!("--{} {}", key, value));
                }
            }
        }

        command_parts.join(" ")
    }

    /// Execute the internal command
    pub async fn invoke(&self, _updates: &mut impl Write) -> Result<InvokeOutput> {
        // Build the command string directly
        let command_string = self.build_command_string();

        // Return the command string directly - it will be processed by HandleInput
        Ok(InvokeOutput {
            output: OutputKind::ExecuteCommand(command_string),
        })
    }

    /// Validate the internal command
    pub async fn validate(&mut self) -> Result<()> {
        // Basic validation - ensure command is not empty
        if self.command.trim().is_empty() {
            return Err(eyre::eyre!("Command cannot be empty"));
        }
        Ok(())
    }

    /// Queue description for the command execution
    pub fn queue_description(&self, updates: &mut impl Write) -> Result<()> {
        let command_string = self.build_command_string();

        queue!(
            updates,
            style::Print(CONTINUATION_LINE),
            style::Print("\n"),
            style::Print(" ● "),
            style::Print("I will run the following command: "),
            style::SetForegroundColor(Color::Yellow),
            style::Print(command_string),
            style::ResetColor,
            style::Print("\n")
        )?;

        // Add the summary if available
        if let Some(ref summary) = self.summary {
            queue!(
                updates,
                style::Print(CONTINUATION_LINE),
                style::Print("\n"),
                style::Print(PURPOSE_ARROW),
                style::SetForegroundColor(Color::Blue),
                style::Print("Purpose: "),
                style::ResetColor,
                style::Print(summary),
                style::Print("\n"),
            )?;
        }

        Ok(())
    }
}

/// Get the tool specification for internal_command
///
/// This function builds the tool specification for the internal_command tool
/// with a comprehensive description of available commands.
pub fn get_tool_spec() -> ToolSpec {
    // Build a comprehensive description that includes all commands
    let mut description = "Tool for suggesting internal Q commands based on user intent. ".to_string();
    description.push_str("This tool helps the AI suggest appropriate commands within the Q chat system ");
    description.push_str("when a user's natural language query indicates they want to perform a specific action.\n\n");
    description.push_str("Available commands:\n");

    // Add basic command information
    let commands = vec![
        ("quit", "Exit the application"),
        ("clear", "Clear the conversation history"),
        ("help", "Show help information"),
        (
            "context",
            "Manage context files (subcommands: add, rm, clear, show, hooks)",
        ),
        (
            "profile",
            "Manage profiles (subcommands: list, create, delete, set, rename, help)",
        ),
        (
            "tools",
            "Manage tools (subcommands: schema, trust, untrust, trustall, reset, help)",
        ),
        ("issue", "Report an issue or request a feature"),
        ("compact", "Compact the conversation history"),
        ("editor", "Open an editor for input"),
        ("usage", "Show token usage statistics"),
        ("prompts", "Manage prompts (subcommands: list, get, help)"),
        ("load", "Load conversation from file"),
        ("save", "Save conversation to file"),
        ("mcp", "Manage MCP servers"),
        ("model", "Manage model settings"),
    ];

    // Add each command to the description
    for (name, desc) in &commands {
        description.push_str(&format!("- {}: {}\n", name, desc));
    }

    // Add information about how to access list data for commands that manage lists
    description.push_str("\nList data access commands:\n");
    description.push_str("- For context files: Use '/context show' to see all current context files\n");
    description.push_str("- For profiles: Use '/profile list' to see all available profiles\n");
    description.push_str("- For tools: Use '/tools schema' to see all available tools and their status\n");
    description.push_str("These commands can be used to dynamically retrieve the current state of lists.\n");

    // Add examples of natural language that should trigger this tool
    description.push_str("\nExamples of natural language that should trigger this tool:\n");
    description.push_str("- \"Clear my conversation\" -> internal_command with command=\"clear\"\n");
    description.push_str(
        "- \"I want to add a file as context\" -> internal_command with command=\"context\", subcommand=\"add\"\n",
    );
    description.push_str(
        "- \"Show me the available profiles\" -> internal_command with command=\"profile\", subcommand=\"list\"\n",
    );
    description.push_str("- \"Exit the application\" -> internal_command with command=\"quit\"\n");
    description.push_str("- \"Add this file to my context\" -> internal_command with command=\"context\", subcommand=\"add\", args=[\"file.txt\"]\n");
    description.push_str(
        "- \"How do I switch profiles?\" -> internal_command with command=\"profile\", subcommand=\"help\"\n",
    );
    description.push_str("- \"I need to report a bug\" -> internal_command with command=\"issue\"\n");
    description.push_str("- \"Let me trust the file write tool\" -> internal_command with command=\"tools\", subcommand=\"trust\", args=[\"fs_write\"]\n");
    description.push_str(
        "- \"Show what tools are available\" -> internal_command with command=\"tools\", subcommand=\"schema\"\n",
    );
    description.push_str("- \"I want to start fresh\" -> internal_command with command=\"clear\"\n");
    description.push_str("- \"Can you help me create a new profile?\" -> internal_command with command=\"profile\", subcommand=\"create\"\n");
    description.push_str("- \"I'd like to see what context files I have\" -> internal_command with command=\"context\", subcommand=\"show\"\n");
    description.push_str("- \"Remove the second context file\" -> internal_command with command=\"context\", subcommand=\"rm\", args=[\"2\"]\n");
    description.push_str(
        "- \"Trust all tools for this session\" -> internal_command with command=\"tools\", subcommand=\"trustall\"\n",
    );
    description.push_str(
        "- \"Reset tool permissions to default\" -> internal_command with command=\"tools\", subcommand=\"reset\"\n",
    );
    description.push_str("- \"I want to compact the conversation\" -> internal_command with command=\"compact\"\n");
    description.push_str("- \"Show me the help for context commands\" -> internal_command with command=\"context\", subcommand=\"help\"\n");
    description.push_str("- \"Show me my token usage\" -> internal_command with command=\"usage\"\n");

    // Create the tool specification
    serde_json::from_value(serde_json::json!({
        "name": "internal_command",
        "description": description,
        "input_schema": {
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command to execute (without the leading slash). Available commands: quit, clear, help, context, profile, tools, issue, compact, editor, usage, prompts, load, save, mcp, model"
                },
                "subcommand": {
                    "type": "string",
                    "description": "Optional subcommand for commands that support them (context, profile, tools, prompts)"
                },
                "args": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    },
                    "description": "Optional arguments for the command"
                },
                "flags": {
                    "type": "object",
                    "additionalProperties": {
                        "type": "string"
                    },
                    "description": "Optional flags for the command"
                },
                "summary": {
                    "type": "string",
                    "description": "A brief explanation of what the command does"
                }
            },
            "required": ["command"]
        }
    })).expect("Failed to create tool spec")
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_internal_command_creation() {
        let cmd = InternalCommand {
            command: "quit".to_string(),
            subcommand: None,
            args: None,
            flags: None,
            summary: None,
        };
        assert_eq!(cmd.command, "quit");
        assert!(cmd.subcommand.is_none());
        assert!(cmd.args.is_none());
        assert!(cmd.flags.is_none());
        assert!(cmd.summary.is_none());
    }

    #[test]
    fn test_internal_command_with_subcommand() {
        let cmd = InternalCommand {
            command: "profile".to_string(),
            subcommand: Some("list".to_string()),
            args: None,
            flags: None,
            summary: None,
        };
        assert_eq!(cmd.command, "profile");
        assert_eq!(cmd.subcommand, Some("list".to_string()));
    }

    #[test]
    fn test_internal_command_with_args() {
        let cmd = InternalCommand {
            command: "context".to_string(),
            subcommand: Some("add".to_string()),
            args: Some(vec!["file.txt".to_string()]),
            flags: None,
            summary: None,
        };
        assert_eq!(cmd.command, "context");
        assert_eq!(cmd.subcommand, Some("add".to_string()));
        assert_eq!(cmd.args, Some(vec!["file.txt".to_string()]));
    }

    #[test]
    fn test_requires_acceptance() {
        // Dangerous commands should require acceptance
        let quit_cmd = InternalCommand {
            command: "quit".to_string(),
            subcommand: None,
            args: None,
            flags: None,
            summary: None,
        };
        assert!(quit_cmd.requires_acceptance());

        let clear_cmd = InternalCommand {
            command: "clear".to_string(),
            subcommand: None,
            args: None,
            flags: None,
            summary: None,
        };
        assert!(clear_cmd.requires_acceptance());

        // Safe/read-only commands should NOT require acceptance
        let help_cmd = InternalCommand {
            command: "help".to_string(),
            subcommand: None,
            args: None,
            flags: None,
            summary: None,
        };
        assert!(!help_cmd.requires_acceptance());

        let profile_list_cmd = InternalCommand {
            command: "profile".to_string(),
            subcommand: Some("list".to_string()),
            args: None,
            flags: None,
            summary: None,
        };
        assert!(!profile_list_cmd.requires_acceptance());

        let context_show_cmd = InternalCommand {
            command: "context".to_string(),
            subcommand: Some("show".to_string()),
            args: None,
            flags: None,
            summary: None,
        };
        assert!(!context_show_cmd.requires_acceptance());

        let tools_schema_cmd = InternalCommand {
            command: "tools".to_string(),
            subcommand: Some("schema".to_string()),
            args: None,
            flags: None,
            summary: None,
        };
        assert!(!tools_schema_cmd.requires_acceptance());

        // Dangerous subcommands should require acceptance
        let context_add_cmd = InternalCommand {
            command: "context".to_string(),
            subcommand: Some("add".to_string()),
            args: Some(vec!["file.txt".to_string()]),
            flags: None,
            summary: None,
        };
        assert!(context_add_cmd.requires_acceptance());
    }
}
