use clap::Subcommand;
use crossterm::execute;
use crossterm::style::{
    self,
    Attribute,
    Color,
};

use crate::cli::ConversationState;
use crate::cli::chat::{
    ChatError,
    ChatSession,
    ChatState,
};
use crate::platform::Context;

#[deny(missing_docs)]
#[derive(Debug, PartialEq, Subcommand)]
pub enum PersistSubcommand {
    /// Save the current conversation
    Save {
        path: String,
        #[arg(short, long)]
        force: bool,
    },
    /// Load a previous conversation
    Load { path: String },
}

impl PersistSubcommand {
    pub async fn execute(self, ctx: &Context, session: &mut ChatSession) -> Result<ChatState, ChatError> {
        macro_rules! tri {
            ($v:expr, $name:expr, $path:expr) => {
                match $v {
                    Ok(v) => v,
                    Err(err) => {
                        execute!(
                            session.stderr,
                            style::SetForegroundColor(Color::Red),
                            style::Print(format!("\nFailed to {} {}: {}\n\n", $name, $path, &err)),
                            style::SetAttribute(Attribute::Reset)
                        )?;

                        return Ok(ChatState::PromptUser {
                            skip_printing_tools: true,
                        });
                    },
                }
            };
        }

        match self {
            Self::Save { path, force } => {
                let contents = tri!(serde_json::to_string_pretty(&session.conversation), "export to", &path);
                if ctx.fs.exists(&path) && !force {
                    execute!(
                        session.stderr,
                        style::SetForegroundColor(Color::Red),
                        style::Print(format!(
                            "\nFile at {} already exists. To overwrite, use -f or --force\n\n",
                            &path
                        )),
                        style::SetAttribute(Attribute::Reset)
                    )?;
                    return Ok(ChatState::PromptUser {
                        skip_printing_tools: true,
                    });
                }
                tri!(ctx.fs.write(&path, contents).await, "export to", &path);

                execute!(
                    session.stderr,
                    style::SetForegroundColor(Color::Green),
                    style::Print(format!("\n✔ Exported conversation state to {}\n\n", &path)),
                    style::SetAttribute(Attribute::Reset)
                )?;
            },
            Self::Load { path } => {
                let contents = tri!(ctx.fs.read_to_string(&path).await, "import from", &path);
                let mut new_state: ConversationState = tri!(serde_json::from_str(&contents), "import from", &path);
                new_state.reload_serialized_state(ctx).await;
                session.conversation = new_state;

                execute!(
                    session.stderr,
                    style::SetForegroundColor(Color::Green),
                    style::Print(format!("\n✔ Imported conversation state from {}\n\n", &path)),
                    style::SetAttribute(Attribute::Reset)
                )?;
            },
        }

        Ok(ChatState::PromptUser {
            skip_printing_tools: true,
        })
    }
}
