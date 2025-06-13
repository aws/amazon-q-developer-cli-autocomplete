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

use std::io::Write;

use clap::Subcommand;
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

use crate::cli::RootSubcommand;
use crate::cli::chat::{
    ChatError,
    ChatState,
};
use crate::database::Database;
use crate::telemetry::TelemetryThread;

/// Q Chat slash commands
#[deny(missing_docs)]
#[derive(Debug, PartialEq, Subcommand)]
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
    Usage,
    Mcp(McpArgs),
    Model(ModelArgs),
    #[command(flatten)]
    Persist(PersistSubcommand),
    #[command(flatten)]
    Root(RootSubcommand),
}

impl SlashCommand {
    pub async fn execute(
        self,
        database: &mut Database,
        telemetry: &TelemetryThread,
        output: &mut impl Write,
    ) -> Result<ChatState, ChatError> {
        match self {
            Self::Clear(args) => args.execute().await,
            Self::Compact(args) => args.execute().await,
            Self::PromptEditor(args) => args.execute().await,
            Self::Quit => Ok(ChatState::Exit),
            Self::Profile(subcommand) => subcommand.execute(output, conversation),
            Self::Context(args) => args.execute(output, conversation).await,
            Self::Tools(args) => args.execute().await,
            Self::Prompts(args) => args.execute().await,
            Self::Usage => usage::execute().await,
            Self::Mcp(args) => args.execute().await,
            Self::Model(args) => args.execute().await,
            Self::Root(subcommand) => subcommand.execute(),
            Self::Hooks(args) => args.execute().await,
            Self::Persist(subcommand) => subcommand.execute().await,
        }
    }
}
