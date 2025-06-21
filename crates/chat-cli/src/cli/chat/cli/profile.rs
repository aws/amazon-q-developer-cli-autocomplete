use clap::Subcommand;
use crossterm::execute;
use crossterm::style::{
    self,
    Attribute,
    Color,
};

use crate::cli::chat::{
    ChatError,
    ChatSession,
    ChatState,
};
use crate::platform::Context;
use crate::util::directories::chat_global_persona_path;

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
    pub async fn execute(self, ctx: &Context, session: &mut ChatSession) -> Result<ChatState, ChatError> {
        let agents = &session.conversation.agents;

        macro_rules! _print_err {
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
                let profiles = agents.agents.values().collect::<Vec<_>>();
                let active_profile = agents.get_active();

                execute!(session.stderr, style::Print("\n"))?;
                for profile in profiles {
                    if active_profile.is_some_and(|p| p == profile) {
                        execute!(
                            session.stderr,
                            style::SetForegroundColor(Color::Green),
                            style::Print("* "),
                            style::Print(&profile.name),
                            style::SetForegroundColor(Color::Reset),
                            style::Print("\n")
                        )?;
                    } else {
                        execute!(
                            session.stderr,
                            style::Print("  "),
                            style::Print(&profile.name),
                            style::Print("\n")
                        )?;
                    }
                }
                execute!(session.stderr, style::Print("\n"))?;
            },
            Self::Rename { .. } | Self::Set { .. } | Self::Delete { .. } | Self::Create { .. } => {
                // As part of the persona implementation, we are disabling the ability to
                // switch / create profile after a session has started.
                // TODO: perhaps revive this after we have a decision on profile create /
                // switch
                let global_path = if let Ok(path) = chat_global_persona_path(ctx) {
                    path.to_str().unwrap_or("default global persona path").to_string()
                } else {
                    "default global persona path".to_string()
                };
                execute!(
                    session.stderr,
                    style::SetForegroundColor(Color::Yellow),
                    style::Print(format!(
                        "Perona / Profile persistance has been disabled. To perform any CRUD on persona / profile, use the default persona under {} as example",
                        global_path
                    )),
                    style::SetAttribute(Attribute::Reset)
                )?;
            },
        }

        Ok(ChatState::PromptUser {
            skip_printing_tools: true,
        })
    }
}
