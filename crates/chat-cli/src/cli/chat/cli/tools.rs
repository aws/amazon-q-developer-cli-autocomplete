use std::collections::HashSet;
use std::io::Write;

use clap::{Args, Subcommand};
use crossterm::style::{Attribute, Color};
use crossterm::{queue, style};

use crate::api_client::model::Tool as FigTool;
use crate::cli::chat::consts::DUMMY_TOOL_NAME;
use crate::cli::chat::context::TrustedCommand;
use crate::cli::chat::tools::ToolOrigin;
use crate::cli::chat::tools::execute::dangerous_patterns;
use crate::cli::chat::{ChatError, ChatSession, ChatState, TRUST_ALL_TEXT};
use crate::os::Os;

#[deny(missing_docs)]
#[derive(Debug, PartialEq, Args)]
pub struct ToolsArgs {
    #[command(subcommand)]
    subcommand: Option<ToolsSubcommand>,
}

impl ToolsArgs {
    pub async fn execute(self, os: &mut Os, session: &mut ChatSession) -> Result<ChatState, ChatError> {
        if let Some(subcommand) = self.subcommand {
            return subcommand.execute(os, session).await;
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
    Trust {
        #[arg(required = true)]
        tool_names: Vec<String>,
    },
    /// Revert a tool or tools to per-request confirmation
    Untrust {
        #[arg(required = true)]
        tool_names: Vec<String>,
    },
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
        #[arg(long, value_name = "PATTERN", num_args = 1.., required_unless_present = "all")]
        command: Vec<String>,
        /// Remove from global configuration instead of current profile
        #[arg(long, short)]
        global: bool,
        /// Remove all trusted command patterns
        #[arg(long, conflicts_with = "command")]
        all: bool,
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
    pub async fn execute(self, os: &mut Os, session: &mut ChatSession) -> Result<ChatState, ChatError> {
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
                            style::Print(format!("Tools '{}' are ", valid_tools.join("', '")))
                        } else {
                            style::Print(format!("Tool '{}' is ", valid_tools[0]))
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
                        style::Print("\n"),
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
                            style::Print(format!("Tools '{}' are ", valid_tools.join("', '")))
                        } else {
                            style::Print(format!("Tool '{}' is ", valid_tools[0]))
                        },
                        style::Print("set to per-request confirmation.\n"),
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
                queue!(session.stderr, style::Print(TRUST_ALL_TEXT), style::Print("\n"))?;
            },
            Self::Reset => {
                session.tool_permissions.reset();
                queue!(
                    session.stderr,
                    style::SetForegroundColor(Color::Green),
                    style::Print("Reset all tools to the default permission levels.\n"),
                    style::SetForegroundColor(Color::Reset),
                )?;
            },
            Self::ResetSingle { tool_name } => {
                if session.tool_permissions.has(&tool_name) || session.tool_permissions.trust_all {
                    session.tool_permissions.reset_tool(&tool_name);
                    queue!(
                        session.stderr,
                        style::SetForegroundColor(Color::Green),
                        style::Print(format!("Reset tool '{}' to the default permission level.\n", tool_name)),
                        style::SetForegroundColor(Color::Reset),
                    )?;
                } else {
                    queue!(
                        session.stderr,
                        style::SetForegroundColor(Color::Red),
                        style::Print(format!(
                            "Tool '{}' does not exist or is already in default settings.\n",
                            tool_name
                        )),
                        style::SetForegroundColor(Color::Reset),
                    )?;
                }
            },
            Self::Allow { subcommand } => {
                match subcommand {
                    AllowSubcommand::ExecuteBash {
                        command,
                        description,
                        global,
                    } => {
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
                                    match context_manager.add_trusted_command(os, trusted_command, global).await {
                                        Ok(()) => {
                                            successful_commands.push(cmd_pattern);
                                        },
                                        Err(error) => {
                                            failed_commands.push((cmd_pattern, error.to_string()));
                                        },
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
                                        queue!(session.stderr, style::Print(format!("\n  • \"{}\"", cmd)),)?;
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
                                        style::Print(
                                            "\nCommands matching these patterns will not require confirmation before execution."
                                        ),
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
                                        queue!(session.stderr, style::Print(format!("\n  • \"{}\": {}", cmd, error)),)?;
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
                            },
                        }
                    },
                }
            },
            Self::Remove { subcommand } => {
                match subcommand {
                    RemoveSubcommand::ExecuteBash { command, global, all } => {
                        match session.conversation.context_manager {
                            Some(ref mut context_manager) => {
                                if all {
                                    // Clear all trusted commands
                                    let scope = if global { "global" } else { "profile" };

                                    // Get current commands to show what will be cleared
                                    let current_commands = if global {
                                        context_manager.get_trusted_commands(true)
                                    } else {
                                        context_manager.get_trusted_commands(false)
                                    };

                                    if current_commands.trusted_commands.is_empty() {
                                        queue!(
                                            session.stderr,
                                            style::SetForegroundColor(Color::Yellow),
                                            style::Print(format!(
                                                "\nNo trusted commands found in {} configuration.",
                                                scope
                                            )),
                                            style::SetForegroundColor(Color::Reset),
                                        )?;
                                    } else {
                                        match context_manager.clear_trusted_commands(os, global).await {
                                            Ok(()) => {
                                                queue!(
                                                    session.stderr,
                                                    style::SetForegroundColor(Color::Green),
                                                    style::Print(format!(
                                                        "\nSuccessfully cleared {} trusted command pattern{} from {} configuration:",
                                                        current_commands.trusted_commands.len(),
                                                        if current_commands.trusted_commands.len() == 1 {
                                                            ""
                                                        } else {
                                                            "s"
                                                        },
                                                        scope
                                                    )),
                                                    style::SetForegroundColor(Color::Reset),
                                                )?;
                                                for cmd in &current_commands.trusted_commands {
                                                    queue!(
                                                        session.stderr,
                                                        style::Print(format!("\n  • \"{}\"", cmd.command)),
                                                    )?;
                                                }
                                            },
                                            Err(error) => {
                                                queue!(
                                                    session.stderr,
                                                    style::SetForegroundColor(Color::Red),
                                                    style::Print(format!(
                                                        "\nFailed to clear trusted commands: {}",
                                                        error
                                                    )),
                                                    style::SetForegroundColor(Color::Reset),
                                                )?;
                                            },
                                        }
                                    }
                                } else {
                                    // Remove specific commands - existing logic
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
                                        let command_exists = current_commands
                                            .trusted_commands
                                            .iter()
                                            .any(|cmd| cmd.command == *cmd_pattern);

                                        if !command_exists {
                                            not_found_commands.push(cmd_pattern.clone());
                                            continue;
                                        }

                                        // Command exists, try to remove it
                                        match context_manager.remove_trusted_command(os, cmd_pattern, global).await {
                                            Ok(()) => {
                                                successful_removals.push(cmd_pattern.clone());
                                            },
                                            Err(error) => {
                                                failed_removals.push((cmd_pattern.clone(), error.to_string()));
                                            },
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
                                            queue!(session.stderr, style::Print(format!("\n  • \"{}\"", cmd)),)?;
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
                                            queue!(session.stderr, style::Print(format!("\n  • \"{}\"", cmd)),)?;
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
                                                style::Print(format!(
                                                    "\nNo trusted commands configured in {} scope.",
                                                    scope
                                                )),
                                            )?;
                                        } else {
                                            queue!(
                                                session.stderr,
                                                style::Print(format!(
                                                    "\nAvailable trusted commands in {} scope:",
                                                    scope
                                                )),
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
                                } // Close the else block for specific command removal
                            },
                            None => {
                                queue!(
                                    session.stderr,
                                    style::SetForegroundColor(Color::Red),
                                    style::Print("\nContext manager not available. Cannot remove trusted commands."),
                                    style::SetForegroundColor(Color::Reset),
                                )?;
                            },
                        }
                    },
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
        let args = vec!["test", "allow", "execute_bash", "--command", "npm *", "rm test.txt"];

        let cli = TestCli::try_parse_from(args).expect("Failed to parse arguments");

        match cli.tools {
            ToolsSubcommand::Allow { subcommand } => match subcommand {
                AllowSubcommand::ExecuteBash {
                    command,
                    description: _,
                    global: _,
                } => {
                    assert_eq!(command.len(), 2);
                    assert_eq!(command[0], "npm *");
                    assert_eq!(command[1], "rm test.txt");
                },
            },
            _ => panic!("Expected Allow subcommand"),
        }
    }

    #[test]
    fn test_remove_execute_bash_multiple_commands() {
        // Test parsing multiple command patterns for removal
        let args = vec!["test", "remove", "execute_bash", "--command", "npm *", "rm test.txt"];

        let cli = TestCli::try_parse_from(args).expect("Failed to parse arguments");

        match cli.tools {
            ToolsSubcommand::Remove { subcommand } => match subcommand {
                RemoveSubcommand::ExecuteBash {
                    command,
                    global: _,
                    all,
                } => {
                    assert_eq!(command.len(), 2);
                    assert_eq!(command[0], "npm *");
                    assert_eq!(command[1], "rm test.txt");
                    assert!(!all);
                },
            },
            _ => panic!("Expected Remove subcommand"),
        }
    }

    #[test]
    fn test_allow_execute_bash_single_command() {
        // Test parsing single command pattern
        let args = vec!["test", "allow", "execute_bash", "--command", "ls -la"];

        let cli = TestCli::try_parse_from(args).expect("Failed to parse arguments");

        match cli.tools {
            ToolsSubcommand::Allow { subcommand } => match subcommand {
                AllowSubcommand::ExecuteBash {
                    command,
                    description: _,
                    global: _,
                } => {
                    assert_eq!(command.len(), 1);
                    assert_eq!(command[0], "ls -la");
                },
            },
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
    #[test]
    fn test_remove_execute_bash_all_flag() {
        // Test parsing --all flag for removing all trusted commands
        let args = vec!["test", "remove", "execute_bash", "--all"];

        let cli = TestCli::try_parse_from(args).expect("Failed to parse arguments");

        match cli.tools {
            ToolsSubcommand::Remove { subcommand } => match subcommand {
                RemoveSubcommand::ExecuteBash { command, global, all } => {
                    assert!(command.is_empty()); // No specific commands when using --all
                    assert!(!global); // Default is false
                    assert!(all); // --all flag should be true
                },
            },
            _ => panic!("Expected Remove subcommand"),
        }
    }

    #[test]
    fn test_remove_execute_bash_all_flag_global() {
        // Test parsing --all flag with --global
        let args = vec!["test", "remove", "execute_bash", "--all", "--global"];

        let cli = TestCli::try_parse_from(args).expect("Failed to parse arguments");

        match cli.tools {
            ToolsSubcommand::Remove { subcommand } => match subcommand {
                RemoveSubcommand::ExecuteBash { command, global, all } => {
                    assert!(command.is_empty());
                    assert!(global); // --global flag should be true
                    assert!(all); // --all flag should be true
                },
            },
            _ => panic!("Expected Remove subcommand"),
        }
    }
}
#[test]
fn test_slash_command_parsing_allow() {
    use crate::cli::chat::cli::SlashCommand;
    use clap::Parser;

    // Test parsing /tools allow execute_bash --command "pattern"
    let args = vec!["slash_command", "tools", "allow", "execute_bash", "--command", "npm *"];

    let result = SlashCommand::try_parse_from(args);
    assert!(result.is_ok(), "Failed to parse slash command: {:?}", result.err());

    match result.unwrap() {
        SlashCommand::Tools(tools_args) => match tools_args.subcommand {
            Some(ToolsSubcommand::Allow { subcommand }) => match subcommand {
                AllowSubcommand::ExecuteBash {
                    command,
                    description: _,
                    global: _,
                } => {
                    assert_eq!(command.len(), 1);
                    assert_eq!(command[0], "npm *");
                },
            },
            _ => panic!("Expected Allow subcommand, got: {:?}", tools_args.subcommand),
        },
        _ => panic!("Expected Tools command"),
    }
}

#[test]
fn test_slash_command_parsing_remove() {
    use crate::cli::chat::cli::SlashCommand;
    use clap::Parser;

    // Test parsing /tools remove execute_bash --command "pattern"
    let args = vec!["slash_command", "tools", "remove", "execute_bash", "--command", "npm *"];

    let result = SlashCommand::try_parse_from(args);
    assert!(result.is_ok(), "Failed to parse slash command: {:?}", result.err());

    match result.unwrap() {
        SlashCommand::Tools(tools_args) => match tools_args.subcommand {
            Some(ToolsSubcommand::Remove { subcommand }) => match subcommand {
                RemoveSubcommand::ExecuteBash {
                    command,
                    global: _,
                    all,
                } => {
                    assert_eq!(command.len(), 1);
                    assert_eq!(command[0], "npm *");
                    assert!(!all);
                },
            },
            _ => panic!("Expected Remove subcommand, got: {:?}", tools_args.subcommand),
        },
        _ => panic!("Expected Tools command"),
    }
}

#[test]
fn test_slash_command_parsing_multiple_patterns() {
    use crate::cli::chat::cli::SlashCommand;
    use clap::Parser;

    // Test parsing multiple command patterns
    let args = vec![
        "slash_command",
        "tools",
        "allow",
        "execute_bash",
        "--command",
        "npm *",
        "git status",
        "ls -la",
    ];

    let result = SlashCommand::try_parse_from(args);
    assert!(result.is_ok(), "Failed to parse slash command: {:?}", result.err());

    match result.unwrap() {
        SlashCommand::Tools(tools_args) => match tools_args.subcommand {
            Some(ToolsSubcommand::Allow { subcommand }) => match subcommand {
                AllowSubcommand::ExecuteBash {
                    command,
                    description: _,
                    global: _,
                } => {
                    assert_eq!(command.len(), 3);
                    assert_eq!(command[0], "npm *");
                    assert_eq!(command[1], "git status");
                    assert_eq!(command[2], "ls -la");
                },
            },
            _ => panic!("Expected Allow subcommand, got: {:?}", tools_args.subcommand),
        },
        _ => panic!("Expected Tools command"),
    }
}

#[test]
fn test_slash_command_parsing_with_description() {
    use crate::cli::chat::cli::SlashCommand;
    use clap::Parser;

    // Test parsing with description
    let args = vec![
        "slash_command",
        "tools",
        "allow",
        "execute_bash",
        "--command",
        "npm *",
        "--description",
        "Trust npm commands",
    ];

    let result = SlashCommand::try_parse_from(args);
    assert!(result.is_ok(), "Failed to parse slash command: {:?}", result.err());

    match result.unwrap() {
        SlashCommand::Tools(tools_args) => match tools_args.subcommand {
            Some(ToolsSubcommand::Allow { subcommand }) => match subcommand {
                AllowSubcommand::ExecuteBash {
                    command,
                    description,
                    global: _,
                } => {
                    assert_eq!(command.len(), 1);
                    assert_eq!(command[0], "npm *");
                    assert_eq!(description, Some("Trust npm commands".to_string()));
                },
            },
            _ => panic!("Expected Allow subcommand, got: {:?}", tools_args.subcommand),
        },
        _ => panic!("Expected Tools command"),
    }
}

#[test]
fn test_slash_command_parsing_global_flag() {
    use crate::cli::chat::cli::SlashCommand;
    use clap::Parser;

    // Test parsing with global flag
    let args = vec![
        "slash_command",
        "tools",
        "allow",
        "execute_bash",
        "--command",
        "npm *",
        "--global",
    ];

    let result = SlashCommand::try_parse_from(args);
    assert!(result.is_ok(), "Failed to parse slash command: {:?}", result.err());

    match result.unwrap() {
        SlashCommand::Tools(tools_args) => match tools_args.subcommand {
            Some(ToolsSubcommand::Allow { subcommand }) => match subcommand {
                AllowSubcommand::ExecuteBash {
                    command,
                    description: _,
                    global,
                } => {
                    assert_eq!(command.len(), 1);
                    assert_eq!(command[0], "npm *");
                    assert!(global);
                },
            },
            _ => panic!("Expected Allow subcommand, got: {:?}", tools_args.subcommand),
        },
        _ => panic!("Expected Tools command"),
    }
}
#[test]
fn test_input_parsing_simulation() {
    use crate::cli::chat::cli::SlashCommand;
    use clap::Parser;

    // Simulate how the chat session processes input
    let user_input = "/tools allow execute_bash --command \"npm *\"";

    // This mimics the logic in handle_input method
    if let Some(args) = user_input.strip_prefix("/").and_then(shlex::split) {
        let mut args_with_binary = args.clone();
        args_with_binary.insert(0, "slash_command".to_owned());

        let result = SlashCommand::try_parse_from(args_with_binary);
        assert!(
            result.is_ok(),
            "Failed to parse user input '{}': {:?}",
            user_input,
            result.err()
        );

        match result.unwrap() {
            SlashCommand::Tools(tools_args) => match tools_args.subcommand {
                Some(ToolsSubcommand::Allow { subcommand }) => match subcommand {
                    AllowSubcommand::ExecuteBash {
                        command,
                        description: _,
                        global: _,
                    } => {
                        assert_eq!(command.len(), 1);
                        assert_eq!(command[0], "npm *");
                    },
                },
                _ => panic!("Expected Allow subcommand, got: {:?}", tools_args.subcommand),
            },
            _ => panic!("Expected Tools command"),
        }
    } else {
        panic!("Failed to parse input as slash command");
    }
}

#[test]
fn test_input_parsing_simulation_remove() {
    use crate::cli::chat::cli::SlashCommand;
    use clap::Parser;

    // Test the remove command as well
    let user_input = "/tools remove execute_bash --command \"npm *\"";

    if let Some(args) = user_input.strip_prefix("/").and_then(shlex::split) {
        let mut args_with_binary = args.clone();
        args_with_binary.insert(0, "slash_command".to_owned());

        let result = SlashCommand::try_parse_from(args_with_binary);
        assert!(
            result.is_ok(),
            "Failed to parse user input '{}': {:?}",
            user_input,
            result.err()
        );

        match result.unwrap() {
            SlashCommand::Tools(tools_args) => match tools_args.subcommand {
                Some(ToolsSubcommand::Remove { subcommand }) => match subcommand {
                    RemoveSubcommand::ExecuteBash {
                        command,
                        global: _,
                        all,
                    } => {
                        assert_eq!(command.len(), 1);
                        assert_eq!(command[0], "npm *");
                        assert!(!all);
                    },
                },
                _ => panic!("Expected Remove subcommand, got: {:?}", tools_args.subcommand),
            },
            _ => panic!("Expected Tools command"),
        }
    } else {
        panic!("Failed to parse input as slash command");
    }
}

