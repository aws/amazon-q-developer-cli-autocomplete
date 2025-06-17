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
pub mod subscribe;
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

use crate::cli::chat::cli::subscribe::SubscribeArgs;
use crate::cli::chat::cli::usage::UsageArgs;
use crate::cli::chat::{
    ChatError,
    ChatSession,
    ChatState,
};
use crate::database::Database;
use crate::platform::Context;
use crate::telemetry::TelemetryThread;

/// q (Amazon Q Chat)
#[derive(Debug, PartialEq, Parser)]
#[command(color = clap::ColorChoice::Always)]
pub enum SlashCommand {
    /// Quit the application
    #[command(aliases = ["q", "exit"])]
    Quit,
    /// Clear the conversation history
    Clear(ClearArgs),
    /// Manage profiles
    #[command(subcommand)]
    Profile(ProfileSubcommand),
    /// Manage context files and hooks for the chat session
    #[command(subcommand)]
    Context(ContextSubcommand),
    /// Open $EDITOR (defaults to vi) to compose a prompt
    PromptEditor(EditorArgs),
    /// Summarize the conversation to free up context space
    Compact(CompactArgs),
    /// View and manage tools and permissions
    Tools(ToolsArgs),
    /// View and retrieve prompts
    Prompts(PromptsArgs),
    /// View and manage context hooks
    Hooks(HooksArgs),
    /// Show current session's context window usage
    Usage(UsageArgs),
    /// See mcp server loaded
    Mcp(McpArgs),
    /// Select a model for the current conversation session
    Model(ModelArgs),
    /// Upgrade to a Q Developer Pro subscription for increased query limits
    Subscribe(SubscribeArgs),
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
            Self::Quit => Ok(ChatState::Exit),
            Self::Clear(args) => args.execute(session).await,
            Self::Profile(subcommand) => subcommand.execute(ctx, session).await,
            Self::Context(args) => args.execute(ctx, session).await,
            Self::PromptEditor(args) => args.execute(session).await,
            Self::Compact(args) => args.execute(ctx, database, telemetry, session).await,
            Self::Tools(args) => args.execute(session).await,
            Self::Prompts(args) => args.execute(session).await,
            Self::Hooks(args) => args.execute(ctx, session).await,
            Self::Usage(args) => args.execute(ctx, session).await,
            Self::Mcp(args) => args.execute(session).await,
            Self::Model(args) => args.execute(session).await,
            Self::Subscribe(args) => args.execute(database, session).await,
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
