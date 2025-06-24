use clap::Subcommand;
use crossterm::execute;
use crossterm::style::{
    self,
    Color,
};
use tracing::warn;

use crate::cli::chat::{
    ChatError,
    ChatSession,
    ChatState,
};
use crate::os::Os;

#[deny(missing_docs)]
#[derive(Debug, PartialEq, Subcommand)]
#[command(
    before_long_help = "Profiles allow you to organize and manage different sets of context files for different projects or tasks.

Notes
• The \"global\" profile contains context files that are available in all profiles
• The \"default\" profile is used when no profile is specified
• You can switch between profiles to work on different projects
• Each profile maintains its own set of context files"
)]
pub enum ProfileSubcommand {
    /// List all available profiles
    List,
    /// Create a new profile with the specified name
    Create { name: String },
    /// Delete the specified profile
    Delete { name: String },
    /// Switch to the specified profile
    Set { name: String },
    /// Rename a profile
    Rename { old_name: String, new_name: String },
}

impl ProfileSubcommand {
    pub async fn execute(self, os: &Os, session: &mut ChatSession) -> Result<ChatState, ChatError> {
        let Some(context_manager) = &mut session.conversation.context_manager else {
            return Ok(ChatState::PromptUser {
                skip_printing_tools: true,
            });
        };

        macro_rules! print_err {
            ($err:expr) => {
                execute!(
                    session.stderr,
                    style::SetForegroundColor(Color::Red),
                    style::Print(format!("\nError: {}\n\n", $err)),
                    style::SetForegroundColor(Color::Reset)
                )?
            };
        }

        match self {
            Self::List => {
                let profiles = match context_manager.list_profiles(os).await {
                    Ok(profiles) => profiles,
                    Err(e) => {
                        execute!(
                            session.stderr,
                            style::SetForegroundColor(Color::Red),
                            style::Print(format!("\nError listing profiles: {}\n\n", e)),
                            style::SetForegroundColor(Color::Reset)
                        )?;
                        vec![]
                    },
                };

                execute!(session.stderr, style::Print("\n"))?;
                for profile in profiles {
                    if profile == context_manager.current_profile {
                        execute!(
                            session.stderr,
                            style::SetForegroundColor(Color::Green),
                            style::Print("* "),
                            style::Print(&profile),
                            style::SetForegroundColor(Color::Reset),
                            style::Print("\n")
                        )?;
                    } else {
                        execute!(
                            session.stderr,
                            style::Print("  "),
                            style::Print(&profile),
                            style::Print("\n")
                        )?;
                    }
                }
                execute!(session.stderr, style::Print("\n"))?;
            },
            Self::Create { name } => match context_manager.create_profile(os, &name).await {
                Ok(_) => {
                    execute!(
                        session.stderr,
                        style::SetForegroundColor(Color::Green),
                        style::Print(format!("\nCreated profile: {}\n\n", name)),
                        style::SetForegroundColor(Color::Reset)
                    )?;
                    context_manager
                        .switch_profile(os, &name)
                        .await
                        .map_err(|e| warn!(?e, "failed to switch to newly created profile"))
                        .ok();
                },
                Err(e) => print_err!(e),
            },
            Self::Delete { name } => match context_manager.delete_profile(os, &name).await {
                Ok(_) => {
                    execute!(
                        session.stderr,
                        style::SetForegroundColor(Color::Green),
                        style::Print(format!("\nDeleted profile: {}\n\n", name)),
                        style::SetForegroundColor(Color::Reset)
                    )?;
                },
                Err(e) => print_err!(e),
            },
            Self::Set { name } => match context_manager.switch_profile(os, &name).await {
                Ok(_) => {
                    execute!(
                        session.stderr,
                        style::SetForegroundColor(Color::Green),
                        style::Print(format!("\nSwitched to profile: {}\n\n", name)),
                        style::SetForegroundColor(Color::Reset)
                    )?;
                },
                Err(e) => print_err!(e),
            },
            Self::Rename { old_name, new_name } => {
                match context_manager.rename_profile(os, &old_name, &new_name).await {
                    Ok(_) => {
                        execute!(
                            session.stderr,
                            style::SetForegroundColor(Color::Green),
                            style::Print(format!("\nRenamed profile: {} -> {}\n\n", old_name, new_name)),
                            style::SetForegroundColor(Color::Reset)
                        )?;
                    },
                    Err(e) => print_err!(e),
                }
            },
        }

        Ok(ChatState::PromptUser {
            skip_printing_tools: true,
        })
    }
}
