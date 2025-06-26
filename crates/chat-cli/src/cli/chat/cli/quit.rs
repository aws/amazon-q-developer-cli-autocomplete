use clap::Args;

use crate::cli::chat::cli::persist::PersistSubcommand;
use crate::cli::chat::{ChatError, ChatSession, ChatState};
use crate::os::Os;

#[derive(Debug, PartialEq, Args)]
pub struct QuitArgs {
    /// Save the conversation before quitting
    #[arg(long)]
    pub save: Option<String>,
}

impl QuitArgs {
    pub async fn execute(self, os: &Os, session: &mut ChatSession) -> Result<ChatState, ChatError> {
        if let Some(path) = self.save {
            // Save conversation before quitting
            let persist_cmd = PersistSubcommand::Save { path, force: false };
            persist_cmd.execute(os, session).await?;
        }
        Ok(ChatState::Exit)
    }
}
