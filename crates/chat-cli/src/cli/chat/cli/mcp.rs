use clap::Args;
use crossterm::{
    queue,
    style,
};

use crate::cli::chat::{
    ChatError,
    ChatState,
};

#[deny(missing_docs)]
#[derive(Debug, PartialEq, Args)]
pub struct McpArgs;

impl McpArgs {
    pub async fn execute(self) -> Result<ChatState, ChatError> {
        let terminal_width = self.terminal_width();
        let loaded_servers = self.conversation.tool_manager.mcp_load_record.lock().await;
        let still_loading = self
            .conversation
            .tool_manager
            .pending_clients()
            .await
            .into_iter()
            .map(|name| format!(" - {name}\n"))
            .collect::<Vec<_>>()
            .join("");
        for (server_name, msg) in loaded_servers.iter() {
            let msg = msg
                .iter()
                .map(|record| match record {
                    LoadingRecord::Err(content) | LoadingRecord::Warn(content) | LoadingRecord::Success(content) => {
                        content.clone()
                    },
                })
                .collect::<Vec<_>>()
                .join("\n--- tools refreshed ---\n");
            queue!(
                output,
                style::Print(server_name),
                style::Print("\n"),
                style::Print(format!("{}\n", "▔".repeat(terminal_width))),
                style::Print(msg),
                style::Print("\n")
            )?;
        }
        if !still_loading.is_empty() {
            queue!(
                output,
                style::Print("Still loading:\n"),
                style::Print(format!("{}\n", "▔".repeat(terminal_width))),
                style::Print(still_loading),
                style::Print("\n")
            )?;
        }

        output.flush()?;
    }
}
