use std::collections::HashSet;

use clap::Subcommand;
use crossterm::style::{
    Attribute,
    Color,
};
use crossterm::{
    execute,
    style,
};

use crate::cli::chat::consts::CONTEXT_FILES_MAX_SIZE;
use crate::cli::chat::token_counter::TokenCounter;
use crate::cli::chat::util::drop_matched_context_files;
use crate::cli::chat::{
    ChatError,
    ChatSession,
    ChatState,
};
use crate::platform::Context;

#[deny(missing_docs)]
#[derive(Debug, PartialEq, Subcommand)]
#[command(
    before_long_help = "Context rules determine which files are included in your Amazon Q session. 
The files matched by these rules provide Amazon Q with additional information 
about your project or environment. Adding relevant files helps Q generate 
more accurate and helpful responses.

Notes
• You can add specific files or use glob patterns (e.g., \"*.py\", \"src/**/*.js\")
• Profile rules apply only to the current profile
• Global rules apply across all profiles
• Context is preserved between chat sessions"
)]
pub enum ContextSubcommand {
    /// Display the context rule configuration and matched files
    Show {
        /// Print out each matched file's content, hook configurations, and last
        /// session.conversation summary
        expand: bool,
    },
    /// Add context rules (filenames or glob patterns)
    Add {
        /// Add to global rules (available in all profiles)
        global: bool,
        /// Include even if matched files exceed size limits
        force: bool,
        paths: Vec<String>,
    },
    /// Remove specified rules from current profile
    Remove {
        /// Remove specified rules globally
        global: bool,
        paths: Vec<String>,
    },
    /// Remove all rules from current profile
    Clear {
        /// Remove global rules
        global: bool,
    },
}

impl ContextSubcommand {
    pub async fn execute(self, ctx: &Context, session: &mut ChatSession) -> Result<ChatState, ChatError> {
        let Some(context_manager) = &mut session.conversation.context_manager else {
            execute!(
                session.output,
                style::SetForegroundColor(Color::Red),
                style::Print("\nContext management is not available.\n\n"),
                style::SetForegroundColor(Color::Reset)
            )?;

            return Ok(ChatState::PromptUser {
                skip_printing_tools: true,
            });
        };

        match self {
            Self::Show { expand } => {
                // Display global context
                execute!(
                    session.output,
                    style::SetAttribute(Attribute::Bold),
                    style::SetForegroundColor(Color::Magenta),
                    style::Print("\n🌍 global:\n"),
                    style::SetAttribute(Attribute::Reset),
                )?;
                let mut global_context_files = HashSet::new();
                let mut profile_context_files = HashSet::new();
                if context_manager.global_config.paths.is_empty() {
                    execute!(
                        session.output,
                        style::SetForegroundColor(Color::DarkGrey),
                        style::Print("    <none>\n"),
                        style::SetForegroundColor(Color::Reset)
                    )?;
                } else {
                    for path in &context_manager.global_config.paths {
                        execute!(session.output, style::Print(format!("    {} ", path)))?;
                        if let Ok(context_files) = context_manager.get_context_files_by_path(ctx, path).await {
                            execute!(
                                session.output,
                                style::SetForegroundColor(Color::Green),
                                style::Print(format!(
                                    "({} match{})",
                                    context_files.len(),
                                    if context_files.len() == 1 { "" } else { "es" }
                                )),
                                style::SetForegroundColor(Color::Reset)
                            )?;
                            global_context_files.extend(context_files);
                        }
                        execute!(session.output, style::Print("\n"))?;
                    }
                }

                // Display profile context
                execute!(
                    session.output,
                    style::SetAttribute(Attribute::Bold),
                    style::SetForegroundColor(Color::Magenta),
                    style::Print(format!("\n👤 profile ({}):\n", context_manager.current_profile)),
                    style::SetAttribute(Attribute::Reset),
                )?;

                if context_manager.profile_config.paths.is_empty() {
                    execute!(
                        session.output,
                        style::SetForegroundColor(Color::DarkGrey),
                        style::Print("    <none>\n\n"),
                        style::SetForegroundColor(Color::Reset)
                    )?;
                } else {
                    for path in &context_manager.profile_config.paths {
                        execute!(session.output, style::Print(format!("    {} ", path)))?;
                        if let Ok(context_files) = context_manager.get_context_files_by_path(ctx, path).await {
                            execute!(
                                session.output,
                                style::SetForegroundColor(Color::Green),
                                style::Print(format!(
                                    "({} match{})",
                                    context_files.len(),
                                    if context_files.len() == 1 { "" } else { "es" }
                                )),
                                style::SetForegroundColor(Color::Reset)
                            )?;
                            profile_context_files.extend(context_files);
                        }
                        execute!(session.output, style::Print("\n"))?;
                    }
                    execute!(session.output, style::Print("\n"))?;
                }

                if global_context_files.is_empty() && profile_context_files.is_empty() {
                    execute!(
                        session.output,
                        style::SetForegroundColor(Color::DarkGrey),
                        style::Print("No files in the current directory matched the rules above.\n\n"),
                        style::SetForegroundColor(Color::Reset)
                    )?;
                } else {
                    let total = global_context_files.len() + profile_context_files.len();
                    let total_tokens = global_context_files
                        .iter()
                        .map(|(_, content)| TokenCounter::count_tokens(content))
                        .sum::<usize>()
                        + profile_context_files
                            .iter()
                            .map(|(_, content)| TokenCounter::count_tokens(content))
                            .sum::<usize>();
                    execute!(
                        session.output,
                        style::SetForegroundColor(Color::Green),
                        style::SetAttribute(Attribute::Bold),
                        style::Print(format!(
                            "{} matched file{} in use:\n",
                            total,
                            if total == 1 { "" } else { "s" }
                        )),
                        style::SetForegroundColor(Color::Reset),
                        style::SetAttribute(Attribute::Reset)
                    )?;

                    for (filename, content) in &global_context_files {
                        let est_tokens = TokenCounter::count_tokens(content);
                        execute!(
                            session.output,
                            style::Print(format!("🌍 {} ", filename)),
                            style::SetForegroundColor(Color::DarkGrey),
                            style::Print(format!("(~{} tkns)\n", est_tokens)),
                            style::SetForegroundColor(Color::Reset),
                        )?;
                        if expand {
                            execute!(
                                session.output,
                                style::SetForegroundColor(Color::DarkGrey),
                                style::Print(format!("{}\n\n", content)),
                                style::SetForegroundColor(Color::Reset)
                            )?;
                        }
                    }

                    for (filename, content) in &profile_context_files {
                        let est_tokens = TokenCounter::count_tokens(content);
                        execute!(
                            session.output,
                            style::Print(format!("👤 {} ", filename)),
                            style::SetForegroundColor(Color::DarkGrey),
                            style::Print(format!("(~{} tkns)\n", est_tokens)),
                            style::SetForegroundColor(Color::Reset),
                        )?;
                        if expand {
                            execute!(
                                session.output,
                                style::SetForegroundColor(Color::DarkGrey),
                                style::Print(format!("{}\n\n", content)),
                                style::SetForegroundColor(Color::Reset)
                            )?;
                        }
                    }

                    if expand {
                        execute!(session.output, style::Print(format!("{}\n\n", "▔".repeat(3))),)?;
                    }

                    let mut combined_files: Vec<(String, String)> = global_context_files
                        .iter()
                        .chain(profile_context_files.iter())
                        .cloned()
                        .collect();

                    let dropped_files = drop_matched_context_files(&mut combined_files, CONTEXT_FILES_MAX_SIZE).ok();

                    execute!(
                        session.output,
                        style::Print(format!("\nTotal: ~{} tokens\n\n", total_tokens))
                    )?;

                    if let Some(dropped_files) = dropped_files {
                        if !dropped_files.is_empty() {
                            execute!(
                                session.output,
                                style::SetForegroundColor(Color::DarkYellow),
                                style::Print(format!(
                                    "Total token count exceeds limit: {}. The following files will be automatically dropped when interacting with Q. Consider removing them. \n\n",
                                    CONTEXT_FILES_MAX_SIZE
                                )),
                                style::SetForegroundColor(Color::Reset)
                            )?;
                            let total_files = dropped_files.len();

                            let truncated_dropped_files = &dropped_files[..10];

                            for (filename, content) in truncated_dropped_files {
                                let est_tokens = TokenCounter::count_tokens(content);
                                execute!(
                                    session.output,
                                    style::Print(format!("{} ", filename)),
                                    style::SetForegroundColor(Color::DarkGrey),
                                    style::Print(format!("(~{} tkns)\n", est_tokens)),
                                    style::SetForegroundColor(Color::Reset),
                                )?;
                            }

                            if total_files > 10 {
                                execute!(
                                    session.output,
                                    style::Print(format!("({} more files)\n", total_files - 10))
                                )?;
                            }
                        }
                    }

                    execute!(session.output, style::Print("\n"))?;
                }

                // Show last cached session.conversation summary if available, otherwise regenerate it
                if expand {
                    if let Some(summary) = session.conversation.latest_summary() {
                        let border = "═".repeat(session.terminal_width().min(80));
                        execute!(
                            session.output,
                            style::Print("\n"),
                            style::SetForegroundColor(Color::Cyan),
                            style::Print(&border),
                            style::Print("\n"),
                            style::SetAttribute(Attribute::Bold),
                            style::Print("                       CONVERSATION SUMMARY"),
                            style::Print("\n"),
                            style::Print(&border),
                            style::SetAttribute(Attribute::Reset),
                            style::Print("\n\n"),
                            style::Print(&summary),
                            style::Print("\n\n\n")
                        )?;
                    }
                }
            },
            Self::Add { global, force, paths } => {
                match context_manager.add_paths(ctx, paths.clone(), global, force).await {
                    Ok(_) => {
                        let target = if global { "global" } else { "profile" };
                        execute!(
                            session.output,
                            style::SetForegroundColor(Color::Green),
                            style::Print(format!("\nAdded {} path(s) to {} context.\n\n", paths.len(), target)),
                            style::SetForegroundColor(Color::Reset)
                        )?;
                    },
                    Err(e) => {
                        execute!(
                            session.output,
                            style::SetForegroundColor(Color::Red),
                            style::Print(format!("\nError: {}\n\n", e)),
                            style::SetForegroundColor(Color::Reset)
                        )?;
                    },
                }
            },
            Self::Remove { global, paths } => match context_manager.remove_paths(ctx, paths.clone(), global).await {
                Ok(_) => {
                    let target = if global { "global" } else { "profile" };
                    execute!(
                        session.output,
                        style::SetForegroundColor(Color::Green),
                        style::Print(format!(
                            "\nRemoved {} path(s) from {} context.\n\n",
                            paths.len(),
                            target
                        )),
                        style::SetForegroundColor(Color::Reset)
                    )?;
                },
                Err(e) => {
                    execute!(
                        session.output,
                        style::SetForegroundColor(Color::Red),
                        style::Print(format!("\nError: {}\n\n", e)),
                        style::SetForegroundColor(Color::Reset)
                    )?;
                },
            },
            Self::Clear { global } => match context_manager.clear(ctx, global).await {
                Ok(_) => {
                    let target = if global {
                        "global".to_string()
                    } else {
                        format!("profile '{}'", context_manager.current_profile)
                    };
                    execute!(
                        session.output,
                        style::SetForegroundColor(Color::Green),
                        style::Print(format!("\nCleared context for {}\n\n", target)),
                        style::SetForegroundColor(Color::Reset)
                    )?;
                },
                Err(e) => {
                    execute!(
                        session.output,
                        style::SetForegroundColor(Color::Red),
                        style::Print(format!("\nError: {}\n\n", e)),
                        style::SetForegroundColor(Color::Reset)
                    )?;
                },
            },
        }

        Ok(ChatState::PromptUser {
            skip_printing_tools: true,
        })
    }
}
