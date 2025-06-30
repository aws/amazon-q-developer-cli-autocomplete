use std::io::Write;

use crossterm::queue;
use crossterm::style::{
    self,
    Color,
};
use eyre::Result;
use serde::Deserialize;

use crate::cli::chat::tools::{
    InvokeOutput,
    MAX_TOOL_RESPONSE_SIZE,
    OutputKind,
};
use crate::cli::chat::util::truncate_safe;
use crate::cli::chat::{
    CONTINUATION_LINE,
    PURPOSE_ARROW,
};
use crate::cli::chat::context::ProcessedTrustedCommands;
pub mod dangerous_patterns;

pub use dangerous_patterns::*;
use crate::platform::Context;

// Platform-specific modules
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(windows))]
mod unix;
#[cfg(not(windows))]
pub use unix::*;

// Common readonly commands that are safe to execute without user confirmation
pub const READONLY_COMMANDS: &[&str] = &[
    "ls", "cat", "echo", "pwd", "which", "head", "tail", "find", "grep", "dir", "type",
];

#[derive(Debug, Clone, Deserialize)]
pub struct ExecuteCommand {
    pub command: String,
    pub summary: Option<String>,
}

impl ExecuteCommand {
    pub fn requires_acceptance(&self, _ctx: &Context, trusted_commands: Option<&ProcessedTrustedCommands>) -> bool {
        let Some(args) = shlex::split(&self.command) else {
            return true;
        };

        // 1. Check for dangerous patterns first (always require acceptance)
        if check_all_dangerous_patterns(&self.command).is_some() {
            return true;
        }

        // 2. Check user-defined trusted commands
        if let Some(trusted_commands) = trusted_commands {
            if trusted_commands.is_trusted(&self.command) {
                return false;
            }
        }

        // Split commands by pipe and check each one
        let mut current_cmd = Vec::new();
        let mut all_commands = Vec::new();

        for arg in args {
            if arg == "|" {
                if !current_cmd.is_empty() {
                    all_commands.push(current_cmd);
                }
                current_cmd = Vec::new();
            } else if arg.contains("|") {
                // if pipe appears without spacing e.g. `echo myimportantfile|args rm` it won't get
                // parsed out, in this case - we want to verify before running
                return true;
            } else {
                current_cmd.push(arg);
            }
        }
        if !current_cmd.is_empty() {
            all_commands.push(current_cmd);
        }

        // Check if each command in the pipe chain starts with a safe command
        for cmd_args in all_commands {
            match cmd_args.first() {
                // Special casing for `find` so that we support most cases while safeguarding
                // against unwanted mutations
                Some(cmd)
                    if cmd == "find"
                        && cmd_args
                            .iter()
                            .any(|arg| arg.contains("-exec") || arg.contains("-delete")) =>
                {
                    return true;
                },
                Some(cmd) if !READONLY_COMMANDS.contains(&cmd.as_str()) => return true,
                None => return true,
                _ => (),
            }
        }

        false
    }

    pub async fn invoke(&self, output: &mut impl Write) -> Result<InvokeOutput> {
        let output = run_command(&self.command, MAX_TOOL_RESPONSE_SIZE / 3, Some(output)).await?;
        let result = serde_json::json!({
            "exit_status": output.exit_status.unwrap_or(0).to_string(),
            "stdout": output.stdout,
            "stderr": output.stderr,
        });

        Ok(InvokeOutput {
            output: OutputKind::Json(result),
        })
    }

    pub fn queue_description(&self, output: &mut impl Write, trusted_commands: Option<&ProcessedTrustedCommands>) -> Result<()> {
        queue!(output, style::Print("I will run the following shell command: "),)?;

        // TODO: Could use graphemes for a better heuristic
        if self.command.len() > 20 {
            queue!(output, style::Print("\n"),)?;
        }

        queue!(
            output,
            style::SetForegroundColor(Color::Green),
            style::Print(&self.command),
            style::Print("\n"),
            style::ResetColor
        )?;
        
        // Indicate if command is trusted by user configuration
        if let Some(trusted_commands) = trusted_commands {
            if trusted_commands.is_trusted(&self.command) {
                queue!(
                    output,
                    style::Print(CONTINUATION_LINE),
                    style::Print("\n"),
                    style::Print(PURPOSE_ARROW),
                    style::SetForegroundColor(Color::Cyan),
                    style::Print("Trusted by user configuration"),
                    style::ResetColor,
                    style::Print("\n"),
                )?;
            }
        }

        // Add the summary if available
        if let Some(summary) = &self.summary {
            queue!(
                output,
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

        queue!(output, style::Print("\n"))?;

        Ok(())
    }

    pub async fn validate(&mut self, _ctx: &Context) -> Result<()> {
        // TODO: probably some small amount of PATH checking
        Ok(())
    }
}

pub struct CommandResult {
    pub exit_status: Option<i32>,
    /// Truncated stdout
    pub stdout: String,
    /// Truncated stderr
    pub stderr: String,
}

// Helper function to format command output with truncation
pub fn format_output(output: &str, max_size: usize) -> String {
    format!(
        "{}{}",
        truncate_safe(output, max_size),
        if output.len() > max_size { " ... truncated" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_requires_acceptance_for_windows_commands() {
        let cmds = &[
            // Safe Windows commands
            ("dir", false),
            ("type file.txt", false),
            ("echo Hello, world!", false),
            // Potentially dangerous Windows commands
            ("del file.txt", true),
            ("rmdir /s /q folder", true),
            ("rd /s /q folder", true),
            ("format c:", true),
            ("erase file.txt", true),
            ("copy file.txt > important.txt", true),
            ("move file.txt destination", true),
            // Command with pipes
            ("dir | findstr txt", true),
            ("type file.txt | findstr pattern", true),
            // Dangerous piped commands
            ("dir | del", true),
            ("type file.txt | del", true),
        ];

        let ctx = Context::new();
        
        for (cmd, expected) in cmds {
            let tool = serde_json::from_value::<ExecuteCommand>(serde_json::json!({
                "command": cmd,
            }))
            .unwrap();
            assert_eq!(
                tool.requires_acceptance(&ctx, None),
                *expected,
                "expected command: `{}` to have requires_acceptance: `{}`",
                cmd,
                expected
            );
        }
    }

    #[tokio::test]
    async fn test_requires_acceptance_with_trusted_commands() {
        use crate::cli::chat::context::{TrustedCommand, TrustedCommandsConfig, ProcessedTrustedCommands};
        
        let ctx = Context::new();
        
        // Create trusted commands configuration
        let mut trusted_config = TrustedCommandsConfig::default();
        trusted_config.trusted_commands.push(TrustedCommand {
            command: "git*".to_string(),
            description: Some("Trust all git commands".to_string()),
        });
        trusted_config.trusted_commands.push(TrustedCommand {
            command: "npm run build".to_string(),
            description: Some("Trust exact npm run build command".to_string()),
        });
        
        let processed_trusted = ProcessedTrustedCommands::new(trusted_config);
        
        let test_cases = &[
            // Commands that should be trusted by user config
            ("git status", false), // matches "git*"
            ("git commit -m 'test'", false), // matches "git*"
            ("npm run build", false), // exact match
            
            // Commands that should still require acceptance
            ("rm -rf /", true), // dangerous pattern
            ("git status && rm file", true), // dangerous pattern overrides trust
            ("npm run test", true), // doesn't match trusted patterns
            ("docker build .", true), // not in trusted commands
        ];
        
        for (cmd, expected) in test_cases {
            let tool = ExecuteCommand {
                command: cmd.to_string(),
                summary: None,
            };
            
            assert_eq!(
                tool.requires_acceptance(&ctx, Some(&processed_trusted)),
                *expected,
                "expected command: `{}` to have requires_acceptance: `{}`",
                cmd,
                expected
            );
        }
    }

    // Tests for trusted command pattern generation
    #[test]
    fn test_generate_pattern_options_simple_command() {
        use crate::cli::chat::ChatSession;
        let options = ChatSession::generate_pattern_options("cat file.txt");
        assert_eq!(options.len(), 2); // Exact + first word (deduped)
        assert_eq!(options[0].0, "cat file.txt");
        assert_eq!(options[1].0, "cat*"); // First word only
    }

    #[test]
    fn test_generate_pattern_options_git_command() {
        use crate::cli::chat::ChatSession;
        let options = ChatSession::generate_pattern_options("git restore --staged Makefile frontend/ opentofu/");
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].0, "git restore --staged Makefile frontend/ opentofu/");
        assert_eq!(options[1].0, "git restore*");
        assert_eq!(options[2].0, "git*");
    }

    #[test]
    fn test_generate_pattern_options_npm_command() {
        use crate::cli::chat::ChatSession;
        let options = ChatSession::generate_pattern_options("npm run build");
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].0, "npm run build");
        assert_eq!(options[1].0, "npm run*");
        assert_eq!(options[2].0, "npm*"); // First word only
    }

    #[test]
    fn test_generate_pattern_options_single_word() {
        use crate::cli::chat::ChatSession;
        let options = ChatSession::generate_pattern_options("pwd");
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].0, "pwd");
        assert_eq!(options[0].1, "Trust this exact command only");
    }

    #[test]
    fn test_generate_pattern_options_command_with_flags_only() {
        use crate::cli::chat::ChatSession;
        let options = ChatSession::generate_pattern_options("ls -la");
        assert_eq!(options.len(), 2); // Exact + first word (stops at "-la")
        assert_eq!(options[0].0, "ls -la");
        assert_eq!(options[1].0, "ls*"); // First word only (nothing before "-")
    }

    #[test]
    fn test_generate_pattern_options_no_duplicate_patterns() {
        use crate::cli::chat::ChatSession;
        // Test case where --version is a flag, not a subcommand
        let options = ChatSession::generate_pattern_options("docker --version");
        assert_eq!(options.len(), 2); // Exact + first word (stops at "--version")
        assert_eq!(options[0].0, "docker --version");
        assert_eq!(options[1].0, "docker*"); // First word only (nothing before "--")
    }

    #[test]
    fn test_generate_pattern_options_multiple_words_before_flag() {
        use crate::cli::chat::ChatSession;
        // Test case with multiple words before hitting a flag
        let options = ChatSession::generate_pattern_options("git commit -m 'my message'");
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].0, "git commit -m 'my message'"); // Exact
        assert_eq!(options[1].0, "git commit*"); // Everything until "-m"
        assert_eq!(options[2].0, "git*"); // First word only
    }

    #[test]
    fn test_generate_pattern_options_no_flags() {
        use crate::cli::chat::ChatSession;
        // Test case with multiple words but no flags
        let options = ChatSession::generate_pattern_options("rsync source dest backup");
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].0, "rsync source dest backup"); // Exact
        assert_eq!(options[1].0, "rsync source*"); // Everything until "-" (no "-" found, so all args + *)
        assert_eq!(options[2].0, "rsync*"); // First word only
    }
}
