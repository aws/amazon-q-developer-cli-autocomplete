use std::collections::HashSet;
use std::io::Write;

use clap::{
    Args,
    Subcommand,
};
use crossterm::style::{
    Attribute,
    Color,
};
use crossterm::{
    queue,
    style,
};

use crate::api_client::model::Tool as FigTool;
use crate::cli::chat::consts::DUMMY_TOOL_NAME;
use crate::cli::chat::context::TrustedCommand;
use crate::cli::chat::tools::execute::dangerous_patterns;
use crate::cli::chat::tools::ToolOrigin;
use crate::cli::chat::{
    ChatError,
    ChatSession,
    ChatState,
    TRUST_ALL_TEXT,
};
use crate::platform::Context;

#[deny(missing_docs)]
#[derive(Debug, PartialEq, Args)]
pub struct ToolsArgs {
    #[command(subcommand)]
    subcommand: Option<ToolsSubcommand>,
}

impl ToolsArgs {
    pub async fn execute(self, ctx: &Context, session: &mut ChatSession) -> Result<ChatState, ChatError> {
        if let Some(subcommand) = self.subcommand {
            return subcommand.execute(ctx, session).await;
        }

        // No subcommand - print the current tools and their permissions.
        // Determine how to format the output nicely.
        let terminal_width = session.terminal_width();
        let longest = session
            .conversation
            .tools
            .values()
            .flatten()
            .map(|FigTool::ToolSpecification(spec)| spec.name.len())
            .max()
            .unwrap_or(0);

        queue!(
            session.stderr,
            style::Print("\n"),
            style::SetAttribute(Attribute::Bold),
            style::Print({
                // Adding 2 because of "- " preceding every tool name
                let width = longest + 2 - "Tool".len() + 4;
                format!("Tool{:>width$}Permission", "", width = width)
            }),
            style::SetAttribute(Attribute::Reset),
            style::Print("\n"),
            style::Print("▔".repeat(terminal_width)),
        )?;

        let mut origin_tools: Vec<_> = session.conversation.tools.iter().collect();

        // Built in tools always appear first.
        origin_tools.sort_by(|(origin_a, _), (origin_b, _)| match (origin_a, origin_b) {
            (ToolOrigin::Native, _) => std::cmp::Ordering::Less,
            (_, ToolOrigin::Native) => std::cmp::Ordering::Greater,
            (ToolOrigin::McpServer(name_a), ToolOrigin::McpServer(name_b)) => name_a.cmp(name_b),
        });

        for (origin, tools) in origin_tools.iter() {
            let mut sorted_tools: Vec<_> = tools
                .iter()
                .filter(|FigTool::ToolSpecification(spec)| spec.name != DUMMY_TOOL_NAME)
                .collect();

            sorted_tools.sort_by_key(|t| match t {
                FigTool::ToolSpecification(spec) => &spec.name,
            });

            let to_display = sorted_tools
                .iter()
                .fold(String::new(), |mut acc, FigTool::ToolSpecification(spec)| {
                    let width = longest - spec.name.len() + 4;
                    acc.push_str(
                        format!(
                            "- {}{:>width$}{}\n",
                            spec.name,
                            "",
                            session.tool_permissions.display_label(&spec.name),
                            width = width
                        )
                        .as_str(),
                    );
                    
                    // Add trusted commands info for execute_bash
                    if spec.name == "execute_bash" || spec.name == "execute_cmd" {
                        if let Some(ref context_manager) = session.conversation.context_manager {
                            let combined_trusted_commands = context_manager.get_combined_trusted_commands();
                            if !combined_trusted_commands.trusted_commands.is_empty() {
                                acc.push_str("    * trusted by profile configuration: ");
                                let commands: Vec<String> = combined_trusted_commands
                                    .trusted_commands
                                    .iter()
                                    .map(|cmd| format!("\"{}\"", cmd.command))
                                    .collect();
                                acc.push_str(&commands.join(" "));
                                acc.push('\n');
                            }
                        }
                    }
                    acc
                });

            let _ = queue!(
                session.stderr,
                style::SetAttribute(Attribute::Bold),
                style::Print(format!("{}:\n", origin)),
                style::SetAttribute(Attribute::Reset),
                style::Print(to_display),
                style::Print("\n")
            );
        }

        let loading = session.conversation.tool_manager.pending_clients().await;
        if !loading.is_empty() {
            queue!(
                session.stderr,
                style::SetAttribute(Attribute::Bold),
                style::Print("Servers still loading"),
                style::SetAttribute(Attribute::Reset),
                style::Print("\n"),
                style::Print("▔".repeat(terminal_width)),
            )?;
            for client in loading {
                queue!(session.stderr, style::Print(format!(" - {client}")), style::Print("\n"))?;
            }
        }



        queue!(
            session.stderr,
            style::Print("\nTrusted tools will run without confirmation."),
            style::SetForegroundColor(Color::DarkGrey),
            style::Print(format!("\n{}\n", "* Default settings")),
            style::Print("\n💡 Use "),
            style::SetForegroundColor(Color::Green),
            style::Print("/tools help"),
            style::SetForegroundColor(Color::Reset),
            style::SetForegroundColor(Color::DarkGrey),
            style::Print(" to edit permissions.\n\n"),
            style::SetForegroundColor(Color::Reset),
        )?;

        Ok(ChatState::default())
    }
}

#[deny(missing_docs)]
#[derive(Debug, PartialEq, Subcommand)]
#[command(
    before_long_help = "By default, Amazon Q will ask for your permission to use certain tools. You can control which tools you
trust so that no confirmation is required. These settings will last only for this session."
)]
pub enum ToolsSubcommand {
    /// Show the input schema for all available tools
    Schema,
    /// Trust a specific tool or tools for the session
    Trust { tool_names: Vec<String> },
    /// Revert a tool or tools to per-request confirmation
    Untrust { tool_names: Vec<String> },
    /// Trust all tools (equivalent to deprecated /acceptall)
    TrustAll,
    /// Reset all tools to default permission levels
    Reset,
    /// Reset a single tool to default permission level
    ResetSingle { tool_name: String },
    /// Allow trusted commands to run without confirmation
    Allow {
        #[command(subcommand)]
        subcommand: AllowSubcommand,
    },
    /// Remove trusted commands
    Remove {
        #[command(subcommand)]
        subcommand: RemoveSubcommand,
    },
}

#[deny(missing_docs)]
#[derive(Debug, PartialEq, Subcommand)]
pub enum AllowSubcommand {
    /// Add trusted command patterns for execute_bash tool
    #[command(name = "execute_bash")]
    ExecuteBash {
        /// Command patterns to trust (supports * wildcards). Multiple patterns can be specified as separate arguments.
        #[arg(long, value_name = "PATTERN", num_args = 1.., required = true)]
        command: Vec<String>,
        /// Optional description for the trusted commands
        #[arg(long)]
        description: Option<String>,
        /// Add to global configuration instead of current profile
        #[arg(long, short)]
        global: bool,
    },
}

#[deny(missing_docs)]
#[derive(Debug, PartialEq, Subcommand)]
pub enum RemoveSubcommand {
    /// Remove trusted command patterns for execute_bash tool
    #[command(name = "execute_bash")]
    ExecuteBash {
        /// Command patterns to remove (must match exactly). Multiple patterns can be specified as separate arguments.
        #[arg(long, value_name = "PATTERN", num_args = 1.., required = true)]
        command: Vec<String>,
        /// Remove from global configuration instead of current profile
        #[arg(long, short)]
        global: bool,
    },
}

/// Validate a command pattern before adding it to trusted commands.
/// 
/// # Arguments
/// * `pattern` - The command pattern to validate
/// 
/// # Returns
/// A Result indicating if the pattern is valid
fn validate_command_pattern(pattern: &str) -> Result<(), String> {
    // Check if pattern is empty
    if pattern.trim().is_empty() {
        return Err("Command pattern cannot be empty".to_string());
    }
    
    // Check for dangerous patterns that should not be trusted
    if let Some(pattern_match) = dangerous_patterns::check_all_dangerous_patterns(pattern) {
        let reason = match pattern_match.pattern_type {
            dangerous_patterns::DangerousPatternType::Destructive => "destructive command",
            dangerous_patterns::DangerousPatternType::ShellControl => "shell control pattern",
            dangerous_patterns::DangerousPatternType::IoRedirection => "I/O redirection pattern",
        };
        return Err(format!(
            "Command pattern contains potentially dangerous sequence '{}' ({}) and cannot be trusted. \
            Consider using more specific patterns.",
            pattern_match.pattern, reason
        ));
    }
    
    // Warn about overly broad patterns
    if pattern == "*" {
        return Err("Pattern '*' is too broad and would trust all commands. Use more specific patterns.".to_string());
    }
    
    Ok(())
}

impl ToolsSubcommand {
    pub async fn execute(self, ctx: &Context, session: &mut ChatSession) -> Result<ChatState, ChatError> {
        let existing_tools: HashSet<&String> = session
            .conversation
            .tools
            .values()
            .flatten()
            .map(|FigTool::ToolSpecification(spec)| &spec.name)
            .collect();

        match self {
            Self::Schema => {
                let schema_json = serde_json::to_string_pretty(&session.conversation.tool_manager.schema)
                    .map_err(|e| ChatError::Custom(format!("Error converting tool schema to string: {e}").into()))?;
                queue!(session.stderr, style::Print(schema_json), style::Print("\n"))?;
            },
            Self::Trust { tool_names } => {
                let (valid_tools, invalid_tools): (Vec<String>, Vec<String>) = tool_names
                    .into_iter()
                    .partition(|tool_name| existing_tools.contains(tool_name));

                if !invalid_tools.is_empty() {
                    queue!(
                        session.stderr,
                        style::SetForegroundColor(Color::Red),
                        style::Print(format!("\nCannot trust '{}', ", invalid_tools.join("', '"))),
                        if invalid_tools.len() > 1 {
                            style::Print("they do not exist.")
                        } else {
                            style::Print("it does not exist.")
                        },
                        style::SetForegroundColor(Color::Reset),
                    )?;
                }
                if !valid_tools.is_empty() {
                    valid_tools.iter().for_each(|t| session.tool_permissions.trust_tool(t));
                    queue!(
                        session.stderr,
                        style::SetForegroundColor(Color::Green),
                        if valid_tools.len() > 1 {
                            style::Print(format!("\nTools '{}' are ", valid_tools.join("', '")))
                        } else {
                            style::Print(format!("\nTool '{}' is ", valid_tools[0]))
                        },
                        style::Print("now trusted. I will "),
                        style::SetAttribute(Attribute::Bold),
                        style::Print("not"),
                        style::SetAttribute(Attribute::Reset),
                        style::SetForegroundColor(Color::Green),
                        style::Print(format!(
                            " ask for confirmation before running {}.",
                            if valid_tools.len() > 1 {
                                "these tools"
                            } else {
                                "this tool"
                            }
                        )),
                        style::SetForegroundColor(Color::Reset),
                    )?;
                }
            },
            Self::Untrust { tool_names } => {
                let (valid_tools, invalid_tools): (Vec<String>, Vec<String>) = tool_names
                    .into_iter()
                    .partition(|tool_name| existing_tools.contains(tool_name));

                if !invalid_tools.is_empty() {
                    queue!(
                        session.stderr,
                        style::SetForegroundColor(Color::Red),
                        style::Print(format!("\nCannot untrust '{}', ", invalid_tools.join("', '"))),
                        if invalid_tools.len() > 1 {
                            style::Print("they do not exist.")
                        } else {
                            style::Print("it does not exist.")
                        },
                        style::SetForegroundColor(Color::Reset),
                    )?;
                }
                if !valid_tools.is_empty() {
                    valid_tools
                        .iter()
                        .for_each(|t| session.tool_permissions.untrust_tool(t));
                    queue!(
                        session.stderr,
                        style::SetForegroundColor(Color::Green),
                        if valid_tools.len() > 1 {
                            style::Print(format!("\nTools '{}' are ", valid_tools.join("', '")))
                        } else {
                            style::Print(format!("\nTool '{}' is ", valid_tools[0]))
                        },
                        style::Print("set to per-request confirmation."),
                        style::SetForegroundColor(Color::Reset),
                    )?;
                }
            },
            Self::TrustAll => {
                session
                    .conversation
                    .tools
                    .values()
                    .flatten()
                    .for_each(|FigTool::ToolSpecification(spec)| {
                        session.tool_permissions.trust_tool(spec.name.as_str());
                    });
                queue!(session.stderr, style::Print(TRUST_ALL_TEXT),)?;
            },
            Self::Reset => {
                session.tool_permissions.reset();
                queue!(
                    session.stderr,
                    style::SetForegroundColor(Color::Green),
                    style::Print("\nReset all tools to the default permission levels."),
                    style::SetForegroundColor(Color::Reset),
                )?;
            },
            Self::ResetSingle { tool_name } => {
                if session.tool_permissions.has(&tool_name) || session.tool_permissions.trust_all {
                    session.tool_permissions.reset_tool(&tool_name);
                    queue!(
                        session.stderr,
                        style::SetForegroundColor(Color::Green),
                        style::Print(format!("\nReset tool '{}' to the default permission level.", tool_name)),
                        style::SetForegroundColor(Color::Reset),
                    )?;
                } else {
                    queue!(
                        session.stderr,
                        style::SetForegroundColor(Color::Red),
                        style::Print(format!(
                            "\nTool '{}' does not exist or is already in default settings.",
                            tool_name
                        )),
                        style::SetForegroundColor(Color::Reset),
                    )?;
                }
            },
            Self::Allow { subcommand } => {
                match subcommand {
                    AllowSubcommand::ExecuteBash { command, description, global } => {
                        let mut successful_commands = Vec::new();
                        let mut failed_commands = Vec::new();
                        
                        match session.conversation.context_manager {
                            Some(ref mut context_manager) => {
                                for cmd_pattern in command {
                                    // Validate each command pattern
                                    if let Err(error) = validate_command_pattern(&cmd_pattern) {
                                        failed_commands.push((cmd_pattern, error));
                                        continue;
                                    }
                                    
                                    // Create the trusted command
                                    let trusted_command = TrustedCommand {
                                        command: cmd_pattern.clone(),
                                        description: description.clone(),
                                    };
                                    
                                    // Add the trusted command to the configuration
                                    match context_manager.add_trusted_command(
                                        ctx,
                                        trusted_command,
                                        global
                                    ).await {
                                        Ok(()) => {
                                            successful_commands.push(cmd_pattern);
                                        },
                                        Err(error) => {
                                            failed_commands.push((cmd_pattern, error.to_string()));
                                        }
                                    }
                                }
                                
                                // Report results
                                if !successful_commands.is_empty() {
                                    let scope = if global { "global" } else { "profile" };
                                    queue!(
                                        session.stderr,
                                        style::SetForegroundColor(Color::Green),
                                        style::Print(format!(
                                            "\nSuccessfully added {} trusted command pattern{} to {} configuration:",
                                            successful_commands.len(),
                                            if successful_commands.len() == 1 { "" } else { "s" },
                                            scope
                                        )),
                                        style::SetForegroundColor(Color::Reset),
                                    )?;
                                    for cmd in &successful_commands {
                                        queue!(
                                            session.stderr,
                                            style::Print(format!("\n  • \"{}\"", cmd)),
                                        )?;
                                    }
                                    if let Some(desc) = description {
                                        queue!(
                                            session.stderr,
                                            style::SetForegroundColor(Color::DarkGrey),
                                            style::Print(format!("\nDescription: {}", desc)),
                                            style::SetForegroundColor(Color::Reset),
                                        )?;
                                    }
                                    queue!(
                                        session.stderr,
                                        style::SetForegroundColor(Color::DarkGrey),
                                        style::Print("\nCommands matching these patterns will not require confirmation before execution."),
                                        style::SetForegroundColor(Color::Reset),
                                    )?;
                                }
                                
                                if !failed_commands.is_empty() {
                                    queue!(
                                        session.stderr,
                                        style::SetForegroundColor(Color::Red),
                                        style::Print(format!(
                                            "\nFailed to add {} command pattern{}:",
                                            failed_commands.len(),
                                            if failed_commands.len() == 1 { "" } else { "s" }
                                        )),
                                        style::SetForegroundColor(Color::Reset),
                                    )?;
                                    for (cmd, error) in &failed_commands {
                                        queue!(
                                            session.stderr,
                                            style::Print(format!("\n  • \"{}\": {}", cmd, error)),
                                        )?;
                                    }
                                }
                            },
                            None => {
                                queue!(
                                    session.stderr,
                                    style::SetForegroundColor(Color::Red),
                                    style::Print("\nContext manager not available. Cannot add trusted commands."),
                                    style::SetForegroundColor(Color::Reset),
                                )?;
                            }
                        }
                    }
                }
            },
            Self::Remove { subcommand } => {
                match subcommand {
                    RemoveSubcommand::ExecuteBash { command, global } => {
                        match session.conversation.context_manager {
                            Some(ref mut context_manager) => {
                                // Get current trusted commands to check which commands exist
                                let current_commands = if global {
                                    context_manager.get_trusted_commands(true)
                                } else {
                                    context_manager.get_trusted_commands(false)
                                };
                                
                                let mut successful_removals = Vec::new();
                                let mut failed_removals = Vec::new();
                                let mut not_found_commands = Vec::new();
                                
                                // Check each command
                                for cmd_pattern in &command {
                                    let command_exists = current_commands.trusted_commands
                                        .iter()
                                        .any(|cmd| cmd.command == *cmd_pattern);
                                    
                                    if !command_exists {
                                        not_found_commands.push(cmd_pattern.clone());
                                        continue;
                                    }
                                    
                                    // Command exists, try to remove it
                                    match context_manager.remove_trusted_command(ctx, cmd_pattern, global).await {
                                        Ok(()) => {
                                            successful_removals.push(cmd_pattern.clone());
                                        },
                                        Err(error) => {
                                            failed_removals.push((cmd_pattern.clone(), error.to_string()));
                                        }
                                    }
                                }
                                
                                // Report results
                                if !successful_removals.is_empty() {
                                    let scope = if global { "global" } else { "profile" };
                                    queue!(
                                        session.stderr,
                                        style::SetForegroundColor(Color::Green),
                                        style::Print(format!(
                                            "\nSuccessfully removed {} trusted command pattern{} from {} configuration:",
                                            successful_removals.len(),
                                            if successful_removals.len() == 1 { "" } else { "s" },
                                            scope
                                        )),
                                        style::SetForegroundColor(Color::Reset),
                                    )?;
                                    for cmd in &successful_removals {
                                        queue!(
                                            session.stderr,
                                            style::Print(format!("\n  • \"{}\"", cmd)),
                                        )?;
                                    }
                                }
                                
                                if !failed_removals.is_empty() {
                                    queue!(
                                        session.stderr,
                                        style::SetForegroundColor(Color::Red),
                                        style::Print(format!(
                                            "\nFailed to remove {} command pattern{}:",
                                            failed_removals.len(),
                                            if failed_removals.len() == 1 { "" } else { "s" }
                                        )),
                                        style::SetForegroundColor(Color::Reset),
                                    )?;
                                    for (cmd, error) in &failed_removals {
                                        queue!(
                                            session.stderr,
                                            style::Print(format!("\n  • \"{}\": {}", cmd, error)),
                                        )?;
                                    }
                                }
                                
                                if !not_found_commands.is_empty() {
                                    let scope = if global { "global" } else { "profile" };
                                    queue!(
                                        session.stderr,
                                        style::SetForegroundColor(Color::Red),
                                        style::Print(format!(
                                            "\n{} command pattern{} not found in {} configuration:",
                                            not_found_commands.len(),
                                            if not_found_commands.len() == 1 { "" } else { "s" },
                                            scope
                                        )),
                                        style::SetForegroundColor(Color::Reset),
                                    )?;
                                    for cmd in &not_found_commands {
                                        queue!(
                                            session.stderr,
                                            style::Print(format!("\n  • \"{}\"", cmd)),
                                        )?;
                                    }
                                    
                                    // Show available commands if any commands were not found
                                    // Refresh the list to show current state after removals
                                    let updated_commands = if global {
                                        context_manager.get_trusted_commands(true)
                                    } else {
                                        context_manager.get_trusted_commands(false)
                                    };
                                    
                                    if updated_commands.trusted_commands.is_empty() {
                                        queue!(
                                            session.stderr,
                                            style::Print(format!("\nNo trusted commands configured in {} scope.", scope)),
                                        )?;
                                    } else {
                                        queue!(
                                            session.stderr,
                                            style::Print(format!("\nAvailable trusted commands in {} scope:", scope)),
                                        )?;
                                        for cmd in &updated_commands.trusted_commands {
                                            queue!(
                                                session.stderr,
                                                style::Print(format!("\n  • \"{}\"", cmd.command)),
                                            )?;
                                            if let Some(desc) = &cmd.description {
                                                queue!(
                                                    session.stderr,
                                                    style::SetForegroundColor(Color::DarkGrey),
                                                    style::Print(format!(" - {}", desc)),
                                                    style::SetForegroundColor(Color::Reset),
                                                )?;
                                            }
                                        }
                                    }
                                }
                            },
                            None => {
                                queue!(
                                    session.stderr,
                                    style::SetForegroundColor(Color::Red),
                                    style::Print("\nContext manager not available. Cannot remove trusted commands."),
                                    style::SetForegroundColor(Color::Reset),
                                )?;
                            }
                        }
                    }
                }
            },
        };

        session.stderr.flush()?;

        Ok(ChatState::PromptUser {
            skip_printing_tools: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct TestCli {
        #[command(subcommand)]
        tools: ToolsSubcommand,
    }

    #[test]
    fn test_allow_execute_bash_multiple_commands() {
        // Test parsing multiple command patterns as separate arguments
        let args = vec![
            "test",
            "allow",
            "execute_bash", 
            "--command", "npm *", "rm test.txt"
        ];
        
        let cli = TestCli::try_parse_from(args).expect("Failed to parse arguments");
        
        match cli.tools {
            ToolsSubcommand::Allow { subcommand } => {
                match subcommand {
                    AllowSubcommand::ExecuteBash { command, description: _, global: _ } => {
                        assert_eq!(command.len(), 2);
                        assert_eq!(command[0], "npm *");
                        assert_eq!(command[1], "rm test.txt");
                    }
                }
            }
            _ => panic!("Expected Allow subcommand"),
        }
    }

    #[test]
    fn test_remove_execute_bash_multiple_commands() {
        // Test parsing multiple command patterns for removal
        let args = vec![
            "test",
            "remove",
            "execute_bash",
            "--command", "npm *", "rm test.txt"
        ];
        
        let cli = TestCli::try_parse_from(args).expect("Failed to parse arguments");
        
        match cli.tools {
            ToolsSubcommand::Remove { subcommand } => {
                match subcommand {
                    RemoveSubcommand::ExecuteBash { command, global: _ } => {
                        assert_eq!(command.len(), 2);
                        assert_eq!(command[0], "npm *");
                        assert_eq!(command[1], "rm test.txt");
                    }
                }
            }
            _ => panic!("Expected Remove subcommand"),
        }
    }

    #[test]
    fn test_allow_execute_bash_single_command() {
        // Test parsing single command pattern
        let args = vec![
            "test",
            "allow", 
            "execute_bash",
            "--command", "ls -la"
        ];
        
        let cli = TestCli::try_parse_from(args).expect("Failed to parse arguments");
        
        match cli.tools {
            ToolsSubcommand::Allow { subcommand } => {
                match subcommand {
                    AllowSubcommand::ExecuteBash { command, description: _, global: _ } => {
                        assert_eq!(command.len(), 1);
                        assert_eq!(command[0], "ls -la");
                    }
                }
            }
            _ => panic!("Expected Allow subcommand"),
        }
    }

    #[test]
    fn test_validate_command_pattern_valid() {
        // Test valid command patterns
        assert!(validate_command_pattern("npm install").is_ok());
        assert!(validate_command_pattern("ls -la").is_ok());
        assert!(validate_command_pattern("npm *").is_ok());
        assert!(validate_command_pattern("git status").is_ok());
    }

    #[test]
    fn test_validate_command_pattern_dangerous() {
        // Test dangerous patterns are rejected
        assert!(validate_command_pattern("rm -rf /").is_err());
        assert!(validate_command_pattern("ls > file.txt").is_err());
        assert!(validate_command_pattern("cmd && rm file").is_err());
        assert!(validate_command_pattern("$(malicious)").is_err());
    }

    #[test]
    fn test_validate_command_pattern_too_broad() {
        // Test overly broad patterns are rejected
        assert!(validate_command_pattern("*").is_err());
    }

    #[test]
    fn test_validate_command_pattern_empty() {
        // Test empty patterns are rejected
        assert!(validate_command_pattern("").is_err());
        assert!(validate_command_pattern("   ").is_err());
    }
}