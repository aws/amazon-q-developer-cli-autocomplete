use clap::Args;

use crate::cli::chat::{
    ChatError,
    ChatSession,
    ChatState,
};
use crate::os::Os;

#[deny(missing_docs)]
#[derive(Debug, PartialEq, Args)]
#[command(
    before_long_help = "/compact summarizes the conversation history to free up context space
while preserving essential information. This is useful for long-running conversations
that may eventually reach memory constraints.

When to use
• When you see the memory constraint warning message
• When a conversation has been running for a long time
• Before starting a new topic within the same session
• After completing complex tool operations

How it works
• Creates an AI-generated summary of your conversation
• Retains key information, code, and tool executions in the summary
• Clears the conversation history to free up space
• The assistant will reference the summary context in future responses"
)]
pub struct CompactArgs {
    /// The prompt to use when generating the summary
    prompt: Option<String>,
    #[arg(long)]
    show_summary: bool,
}

impl CompactArgs {
    pub async fn execute(self, os: &Os, session: &mut ChatSession) -> Result<ChatState, ChatError> {
        session.compact_history(os, self.prompt, self.show_summary).await
    }
}
