use clap::Subcommand;
use crossterm::style::{
    Attribute,
    Color,
};
use crossterm::{
    execute,
    style,
};

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
        #[arg(long)]
        expand: bool,
    },
    /// Add context rules (filenames or glob patterns)
    Add {
        /// Include even if matched files exceed size limits
        #[arg(short, long)]
        force: bool,
        paths: Vec<String>,
    },
    /// Remove specified rules from current profile
    Remove { paths: Vec<String> },
    /// Remove all rules from current profile
    Clear,
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
                        }
                        execute!(session.output, style::Print("\n"))?;
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
            Self::Add { force, paths } => match context_manager.add_paths(ctx, paths.clone(), force).await {
                Ok(_) => {
                    execute!(
                        session.output,
                        style::SetForegroundColor(Color::Green),
                        style::Print(format!("\nAdded {} path(s) to context.\n\n", paths.len())),
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
            Self::Remove { paths } => match context_manager.remove_paths(paths.clone()) {
                Ok(_) => {
                    execute!(
                        session.output,
                        style::SetForegroundColor(Color::Green),
                        style::Print(format!("\nRemoved {} path(s) from context.\n\n", paths.len(),)),
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
            Self::Clear => {
                context_manager.clear();
                execute!(
                    session.output,
                    style::SetForegroundColor(Color::Green),
                    style::Print(format!("\nCleared context\n\n")),
                    style::SetForegroundColor(Color::Reset)
                )?;
            },
        }

        Ok(ChatState::PromptUser {
            skip_printing_tools: true,
        })
    }
}
