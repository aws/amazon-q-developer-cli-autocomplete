pub mod clear;
pub mod compact;
pub mod context;
pub mod editor;
pub mod hooks;
pub mod mcp;
pub mod model;
pub mod persist;
pub mod profile;
pub mod prompts;
pub mod tools;
pub mod usage;

use clap::Parser;
use clear::ClearArgs;
use compact::CompactArgs;
use context::ContextSubcommand;
use editor::EditorArgs;
use hooks::HooksArgs;
use mcp::McpArgs;
use model::ModelArgs;
use persist::PersistSubcommand;
use profile::ProfileSubcommand;
use prompts::PromptsArgs;
use tools::ToolsArgs;

use crate::cli::chat::cli::usage::UsageArgs;
use crate::cli::chat::{
    ChatError,
    ChatSession,
    ChatState,
};
use crate::database::Database;
use crate::platform::Context;
use crate::telemetry::TelemetryThread;

/// Q Chat slash commands
#[deny(missing_docs)]
#[derive(Debug, PartialEq, Parser)]
pub enum SlashCommand {
    /// Exit the chat
    #[command(aliases = ["q", "exit"])]
    Quit,
    /// Clear the current conversation and start fresh
    Clear(ClearArgs),
    /// Modify
    #[command(subcommand)]
    Profile(ProfileSubcommand),
    #[command(subcommand)]
    Context(ContextSubcommand),
    PromptEditor(EditorArgs),
    Compact(CompactArgs),
    Tools(ToolsArgs),
    Prompts(PromptsArgs),
    Hooks(HooksArgs),
    Usage(UsageArgs),
    Mcp(McpArgs),
    Model(ModelArgs),
    #[command(flatten)]
    Persist(PersistSubcommand),
    // #[command(flatten)]
    // Root(RootSubcommand),
}

impl SlashCommand {
    pub async fn execute(
        self,
        ctx: &mut Context,
        database: &mut Database,
        telemetry: &TelemetryThread,
        session: &mut ChatSession,
    ) -> Result<ChatState, ChatError> {
        match self {
            Self::Clear(args) => args.execute(session).await,
            Self::Compact(args) => args.execute(ctx, database, telemetry, session).await,
            Self::PromptEditor(args) => args.execute(session).await,
            Self::Quit => Ok(ChatState::Exit),
            Self::Profile(subcommand) => subcommand.execute(ctx, session).await,
            Self::Context(args) => args.execute(ctx, session).await,
            Self::Tools(args) => args.execute(session).await,
            Self::Prompts(args) => args.execute(session).await,
            Self::Usage(args) => args.execute(ctx, session).await,
            Self::Mcp(args) => args.execute(session).await,
            Self::Model(args) => args.execute(session).await,
            Self::Hooks(args) => args.execute(ctx, session).await,
            Self::Persist(subcommand) => subcommand.execute(ctx, session).await,
            // Self::Root(subcommand) => {
            //     if let Err(err) = subcommand.execute(ctx, database, telemetry).await {
            //         return Err(ChatError::Custom(err.to_string().into()));
            //     }
            //
            //     Ok(ChatState::PromptUser {
            //         skip_printing_tools: true,
            //     })
            // },
        }
    }
}
