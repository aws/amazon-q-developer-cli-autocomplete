use std::sync::Arc;

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
    ChatState,
};

#[deny(missing_docs)]
#[derive(Debug, PartialEq, Subcommand)]
pub enum PersistSubcommand {
    /// Save the current conversation
    Save { path: String, force: bool },
    /// Load a previous conversation
    Load { path: String },
}

impl PersistSubcommand {
    pub async fn execute(self) -> Result<ChatState, ChatError> {
        macro_rules! tri {
            ($v:expr, $name:expr) => {
                match $v {
                    Ok(v) => v,
                    Err(err) => {
                        execute!(
                            output,
                            style::SetForegroundColor(Color::Red),
                            style::Print(format!("\nFailed to {} {}: {}\n\n", $name, &path, &err)),
                            style::SetAttribute(Attribute::Reset)
                        )?;

                        return Ok(ChatState::PromptUser {
                            tool_uses: Some(tool_uses),
                            pending_tool_index,
                            skip_printing_tools: true,
                        });
                    },
                }
            };
        }

        match self {
            Self::Save { path, force } => {
                let contents = tri!(serde_json::to_string_pretty(&self.conversation), "export to");
                if self.ctx.fs.exists(&path) && !force {
                    execute!(
                        output,
                        style::SetForegroundColor(Color::Red),
                        style::Print(format!(
                            "\nFile at {} already exists. To overwrite, use -f or --force\n\n",
                            &path
                        )),
                        style::SetAttribute(Attribute::Reset)
                    )?;
                    return Ok(ChatState::PromptUser {
                        tool_uses: Some(tool_uses),
                        pending_tool_index,
                        skip_printing_tools: true,
                    });
                }
                tri!(self.ctx.fs.write(&path, contents).await, "export to");

                execute!(
                    output,
                    style::SetForegroundColor(Color::Green),
                    style::Print(format!("\n✔ Exported conversation state to {}\n\n", &path)),
                    style::SetAttribute(Attribute::Reset)
                )?;
            },
            Self::Load { path } => {
                let contents = tri!(self.ctx.fs.read_to_string(&path).await, "import from");
                let mut new_state: ConversationState = tri!(serde_json::from_str(&contents), "import from");
                new_state
                    .reload_serialized_state(Arc::clone(&self.ctx), Some(output.clone()))
                    .await;
                self.conversation = new_state;

                execute!(
                    output,
                    style::SetForegroundColor(Color::Green),
                    style::Print(format!("\n✔ Imported conversation state from {}\n\n", &path)),
                    style::SetAttribute(Attribute::Reset)
                )?;
            },
        }

        Ok(ChatState::PromptUser {
            tool_uses: None,
            pending_tool_index: None,
            skip_printing_tools: true,
        })
    }
}
