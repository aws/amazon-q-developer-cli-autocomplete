//! Help text generation for chat commands

#[derive(Debug, Clone)]
pub struct CommandHelp {
    pub command: &'static str,
    pub description: &'static str,
    pub subcommands: &'static [SubCommand],
    pub supported_os: &'static [&'static str], // "windows", "unix", "all"
}

#[derive(Debug, Clone)]
pub struct SubCommand {
    pub name: &'static str,
    pub description: &'static str,
}

/// Help text for the compact command
pub fn compact_help_text() -> String {
    color_print::cformat!(
        r#"
<magenta,em>Conversation Compaction</magenta,em>

The <em>/compact</em> command summarizes the conversation history to free up context space
while preserving essential information. This is useful for long-running conversations
that may eventually reach memory constraints.

<cyan!>Usage</cyan!>
  <em>/compact</em>                   <black!>Summarize the conversation and clear history</black!>
  <em>/compact [prompt]</em>          <black!>Provide custom guidance for summarization</black!>

<cyan!>When to use</cyan!>
• When you see the memory constraint warning message
• When a conversation has been running for a long time
• Before starting a new topic within the same session
• After completing complex tool operations

<cyan!>How it works</cyan!>
• Creates an AI-generated summary of your conversation
• Retains key information, code, and tool executions in the summary
• Clears the conversation history to free up space
• The assistant will reference the summary context in future responses
"#
    )
}

pub const HELP_COMMANDS: &[CommandHelp] = &[
    CommandHelp {
        command: "/clear",
        description: "Clear the conversation history",
        subcommands: &[],
        supported_os: &["all"],
    },
    CommandHelp {
        command: "/issue",
        description: "Report an issue or make a feature request",
        subcommands: &[],
        supported_os: &["all"],
    },
    CommandHelp {
        command: "/editor",
        description: "Open $EDITOR (defaults to vi) to compose a prompt",
        subcommands: &[],
        supported_os: &["unix"],
    },
    CommandHelp {
        command: "/help",
        description: "Show this help dialogue",
        subcommands: &[],
        supported_os: &["all"],
    },
    CommandHelp {
        command: "/quit",
        description: "Quit the application",
        subcommands: &[],
        supported_os: &["all"],
    },
    CommandHelp {
        command: "/compact",
        description: "Summarize the conversation to free up context space",
        subcommands: &[
            SubCommand {
                name: "help",
                description: "Show help for the compact command",
            },
            SubCommand {
                name: "[prompt]",
                description: "Optional custom prompt to guide summarization",
            },
        ],
        supported_os: &["all"],
    },
    CommandHelp {
        command: "/tools",
        description: "View and manage tools and permissions",
        subcommands: &[
            SubCommand {
                name: "help",
                description: "Show an explanation for the trust command",
            },
            SubCommand {
                name: "trust",
                description: "Trust a specific tool or tools for the session",
            },
            SubCommand {
                name: "untrust",
                description: "Revert a tool or tools to per-request confirmation",
            },
            SubCommand {
                name: "trustall",
                description: "Trust all tools (equivalent to deprecated /acceptall)",
            },
            SubCommand {
                name: "reset",
                description: "Reset all tools to default permission levels",
            },
        ],
        supported_os: &["all"],
    },
    CommandHelp {
        command: "/mcp",
        description: "See mcp server loaded",
        subcommands: &[],
        supported_os: &["all"],
    },
    CommandHelp {
        command: "/model",
        description: "Select a model for the current conversation session",
        subcommands: &[],
        supported_os: &["all"],
    },
    CommandHelp {
        command: "/profile",
        description: "Manage profiles",
        subcommands: &[
            SubCommand {
                name: "help",
                description: "Show profile help",
            },
            SubCommand {
                name: "list",
                description: "List profiles",
            },
            SubCommand {
                name: "set",
                description: "Set the current profile",
            },
            SubCommand {
                name: "create",
                description: "Create a new profile",
            },
            SubCommand {
                name: "delete",
                description: "Delete a profile",
            },
            SubCommand {
                name: "rename",
                description: "Rename a profile",
            },
        ],
        supported_os: &["all"],
    },
    CommandHelp {
        command: "/prompts",
        description: "View and retrieve prompts",
        subcommands: &[
            SubCommand {
                name: "help",
                description: "Show prompts help",
            },
            SubCommand {
                name: "list",
                description: "List or search available prompts",
            },
            SubCommand {
                name: "get",
                description: "Retrieve and send a prompt",
            },
        ],
        supported_os: &["all"],
    },
    CommandHelp {
        command: "/context",
        description: "Manage context files and hooks for the chat session",
        subcommands: &[
            SubCommand {
                name: "help",
                description: "Show context help",
            },
            SubCommand {
                name: "show",
                description: "Display current context rules configuration [--expand]",
            },
            SubCommand {
                name: "add",
                description: "Add file(s) to context [--global] [--force]",
            },
            SubCommand {
                name: "rm",
                description: "Remove file(s) from context [--global]",
            },
            SubCommand {
                name: "clear",
                description: "Clear all files from current context [--global]",
            },
            SubCommand {
                name: "hooks",
                description: "View and manage context hooks",
            },
        ],
        supported_os: &["all"],
    },
    CommandHelp {
        command: "/usage",
        description: "Show current session's context window usage",
        subcommands: &[],
        supported_os: &["all"],
    },
    CommandHelp {
        command: "/load",
        description: "Load conversation state from a JSON file",
        subcommands: &[],
        supported_os: &["all"],
    },
    CommandHelp {
        command: "/save",
        description: "Save conversation state to a JSON file",
        subcommands: &[],
        supported_os: &["all"],
    },
    CommandHelp {
        command: "/subscribe",
        description: "Upgrade to a Q Developer Pro subscription for increased query limits",
        subcommands: &[SubCommand {
            name: "manage",
            description: "View and manage your existing subscription on AWS",
        }],
        supported_os: &["all"],
    },
];

pub fn generate_help_text() -> String {
    let current_os = if cfg!(windows) {
        "windows"
    } else if cfg!(unix) {
        "unix"
    } else {
        "all"
    };

    let mut help_text = String::new();
    help_text.push_str("\n\n");
    help_text.push_str(&color_print::cformat!("<magenta,em>q</magenta,em> (Amazon Q Chat)\n\n"));
    help_text.push_str(&color_print::cformat!("<cyan,em>Commands:</cyan,em>\n"));

    for cmd in HELP_COMMANDS {
        // Check if this command is supported on the current OS
        if !cmd.supported_os.contains(&"all") && !cmd.supported_os.contains(&current_os) {
            continue;
        }

        help_text.push_str(&color_print::cformat!(
            "<em>{}</em>        <black!>{}</black!>\n",
            cmd.command,
            cmd.description
        ));

        // Add subcommands
        for subcmd in cmd.subcommands {
            help_text.push_str(&color_print::cformat!(
                "  <em>{}</em>        <black!>{}</black!>\n",
                subcmd.name,
                subcmd.description
            ));
        }
    }

    help_text.push_str(&color_print::cformat!("\n<cyan,em>MCP:</cyan,em>\n"));
    help_text.push_str(&color_print::cformat!("<black!>You can now configure the Amazon Q CLI to use MCP servers. \\nLearn how: https://docs.aws.amazon.com/en_us/amazonq/latest/qdeveloper-ug/command-line-mcp.html</black!>\n\n"));

    help_text.push_str(&color_print::cformat!("<cyan,em>Tips:</cyan,em>\n"));
    help_text.push_str(&color_print::cformat!(
        "<em>!{{command}}</em>            <black!>Quickly execute a command in your current session</black!>\n"
    ));
    help_text.push_str(&color_print::cformat!("<em>Ctrl(^) + j</em>           <black!>Insert new-line to provide multi-line prompt. Alternatively, [Alt(⌥) + Enter(⏎)]</black!>\n"));
    help_text.push_str(&color_print::cformat!("<em>Ctrl(^) + s</em>           <black!>Fuzzy search commands and context files. Use Tab to select multiple items.</black!>\n"));
    help_text.push_str(&color_print::cformat!("                      <black!>Change the keybind to ctrl+x with: q settings chat.skimCommandKey x (where x is any key)</black!>\n"));
    help_text.push_str(&color_print::cformat!("<em>chat.editMode</em>         <black!>Set editing mode (vim or emacs) using: q settings chat.editMode vi/emacs</black!>\n\n"));

    help_text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_help_commands_structure() {
        // Test that all commands have valid structure
        for cmd in HELP_COMMANDS {
            assert!(!cmd.command.is_empty());
            assert!(!cmd.description.is_empty());
            assert!(!cmd.supported_os.is_empty());

            // Ensure supported_os contains valid values
            for os in cmd.supported_os {
                assert!(
                    *os == "all" || *os == "windows" || *os == "unix",
                    "Invalid OS specifier: {}",
                    os
                );
            }
        }
    }

    #[test]
    fn test_editor_command_os_restriction() {
        // Find the /editor command
        let editor_cmd = HELP_COMMANDS
            .iter()
            .find(|cmd| cmd.command == "/editor")
            .expect("/editor command should exist");

        // Verify it's restricted to unix only
        assert_eq!(editor_cmd.supported_os, &["unix"]);
        assert!(!editor_cmd.supported_os.contains(&"windows"));
        assert!(!editor_cmd.supported_os.contains(&"all"));
    }

    #[test]
    fn test_generate_help_text_contains_basic_commands() {
        let help_text = generate_help_text();

        // These commands should always be present regardless of OS
        assert!(help_text.contains("/clear"));
        assert!(help_text.contains("/help"));
        assert!(help_text.contains("/quit"));
        assert!(help_text.contains("/issue"));
        assert!(help_text.contains("/compact"));
        assert!(help_text.contains("/tools"));
        assert!(help_text.contains("/model"));

        // Check that basic structure is present
        assert!(help_text.contains("Amazon Q Chat"));
        assert!(help_text.contains("Commands:"));
        assert!(help_text.contains("Tips:"));
        assert!(help_text.contains("MCP:"));
    }

    #[cfg(windows)]
    #[test]
    fn test_generate_help_text_excludes_editor_on_windows() {
        let help_text = generate_help_text();

        // /editor command should not be present on Windows
        assert!(!help_text.contains("/editor"));
        assert!(!help_text.contains("Open $EDITOR"));
    }

    #[cfg(unix)]
    #[test]
    fn test_generate_help_text_includes_editor_on_unix() {
        let help_text = generate_help_text();

        // /editor command should be present on Unix systems
        assert!(help_text.contains("/editor"));
        assert!(help_text.contains("Open $EDITOR"));
    }

    #[test]
    fn test_all_commands_have_descriptions() {
        for cmd in HELP_COMMANDS {
            assert!(
                !cmd.description.is_empty(),
                "Command {} has no description",
                cmd.command
            );

            // Test subcommands too
            for subcmd in cmd.subcommands {
                assert!(!subcmd.name.is_empty(), "Subcommand has empty name for {}", cmd.command);
                assert!(
                    !subcmd.description.is_empty(),
                    "Subcommand {} has no description for {}",
                    subcmd.name,
                    cmd.command
                );
            }
        }
    }

    #[test]
    fn test_command_format_consistency() {
        for cmd in HELP_COMMANDS {
            // All commands should start with /
            assert!(
                cmd.command.starts_with('/'),
                "Command {} should start with /",
                cmd.command
            );

            // Commands should not contain newlines
            assert!(!cmd.command.contains('\n'), "Command {} contains newline", cmd.command);
            assert!(
                !cmd.description.contains('\n'),
                "Description for {} contains newline",
                cmd.command
            );
        }
    }

    #[test]
    fn test_generate_help_text_output_format() {
        let help_text = generate_help_text();

        // Should start with newlines for proper formatting
        assert!(help_text.starts_with("\n\n"));

        // Should end with newlines for proper formatting
        assert!(help_text.ends_with("\n\n"));

        // Should not be empty
        assert!(!help_text.trim().is_empty());

        // Should contain color formatting codes (from color_print)
        // Note: These are ANSI escape sequences that color_print generates
        assert!(help_text.len() > 100); // Reasonable minimum length
    }
}
