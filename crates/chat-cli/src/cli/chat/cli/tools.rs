use std::collections::HashSet;
use std::io::Write;

use clap::{
    Args,
    Subcommand,
};
use crossterm::style::{
    Attribute,
    Color,
};
use crossterm::{
    queue,
    style,
};

use crate::api_client::model::Tool as FigTool;
use crate::cli::chat::consts::DUMMY_TOOL_NAME;
use crate::cli::chat::tools::ToolOrigin;
use crate::cli::chat::{
    ChatError,
    ChatSession,
    ChatState,
    TRUST_ALL_TEXT,
};

#[deny(missing_docs)]
#[derive(Debug, PartialEq, Args)]
pub struct ToolsArgs {
    #[command(subcommand)]
    subcommand: Option<ToolsSubcommand>,
}

impl ToolsArgs {
    pub async fn execute(self, session: &mut ChatSession) -> Result<ChatState, ChatError> {
        if let Some(subcommand) = self.subcommand {
            return subcommand.execute(session).await;
        }

        // No subcommand - print the current tools and their permissions.
        // Determine how to format the output nicely.
        let terminal_width = session.terminal_width();
        let longest = session
            .conversation
            .tools
            .values()
            .flatten()
            .map(|FigTool::ToolSpecification(spec)| spec.name.len())
            .max()
            .unwrap_or(0);

        queue!(
            session.stderr,
            style::Print("\n"),
            style::SetAttribute(Attribute::Bold),
            style::Print({
                // Adding 2 because of "- " preceding every tool name
                let width = longest + 2 - "Tool".len() + 4;
                format!("Tool{:>width$}Permission", "", width = width)
            }),
            style::SetAttribute(Attribute::Reset),
            style::Print("\n"),
            style::Print("▔".repeat(terminal_width)),
        )?;

        let mut origin_tools: Vec<_> = session.conversation.tools.iter().collect();

        // Built in tools always appear first.
        origin_tools.sort_by(|(origin_a, _), (origin_b, _)| match (origin_a, origin_b) {
            (ToolOrigin::Native, _) => std::cmp::Ordering::Less,
            (_, ToolOrigin::Native) => std::cmp::Ordering::Greater,
            (ToolOrigin::McpServer(name_a), ToolOrigin::McpServer(name_b)) => name_a.cmp(name_b),
        });

        for (origin, tools) in origin_tools.iter() {
            let mut sorted_tools: Vec<_> = tools
                .iter()
                .filter(|FigTool::ToolSpecification(spec)| spec.name != DUMMY_TOOL_NAME)
                .collect();

            sorted_tools.sort_by_key(|t| match t {
                FigTool::ToolSpecification(spec) => &spec.name,
            });

            let to_display = sorted_tools
                .iter()
                .fold(String::new(), |mut acc, FigTool::ToolSpecification(spec)| {
                    let width = longest - spec.name.len() + 4;
                    acc.push_str(
                        format!(
                            "- {}{:>width$}{}\n",
                            spec.name,
                            "",
                            session.tool_permissions.display_label(&spec.name),
                            width = width
                        )
                        .as_str(),
                    );
                    acc
                });

            let _ = queue!(
                session.stderr,
                style::SetAttribute(Attribute::Bold),
                style::Print(format!("{}:\n", origin)),
                style::SetAttribute(Attribute::Reset),
                style::Print(to_display),
                style::Print("\n")
            );
        }

        let loading = session.conversation.tool_manager.pending_clients().await;
        if !loading.is_empty() {
            queue!(
                session.stderr,
                style::SetAttribute(Attribute::Bold),
                style::Print("Servers still loading"),
                style::SetAttribute(Attribute::Reset),
                style::Print("\n"),
                style::Print("▔".repeat(terminal_width)),
            )?;
            for client in loading {
                queue!(session.stderr, style::Print(format!(" - {client}")), style::Print("\n"))?;
            }
        }

        queue!(
            session.stderr,
            style::Print("\nTrusted tools will run without confirmation."),
            style::SetForegroundColor(Color::DarkGrey),
            style::Print(format!("\n{}\n", "* Default settings")),
            style::Print("\n💡 Use "),
            style::SetForegroundColor(Color::Green),
            style::Print("/tools help"),
            style::SetForegroundColor(Color::Reset),
            style::SetForegroundColor(Color::DarkGrey),
            style::Print(" to edit permissions.\n\n"),
            style::SetForegroundColor(Color::Reset),
        )?;

        Ok(ChatState::default())
    }
}

#[deny(missing_docs)]
#[derive(Debug, PartialEq, Subcommand)]
#[command(
    before_long_help = "By default, Amazon Q will ask for your permission to use certain tools. You can control which tools you
trust so that no confirmation is required. These settings will last only for this session."
)]
pub enum ToolsSubcommand {
    /// Show the input schema for all available tools
    Schema,
    /// Trust a specific tool or tools for the session
    Trust {
        #[arg(required = true)]
        tool_names: Vec<String>,
    },
    /// Revert a tool or tools to per-request confirmation
    Untrust {
        #[arg(required = true)]
        tool_names: Vec<String>,
    },
    /// Trust all tools (equivalent to deprecated /acceptall)
    TrustAll,
    /// Reset all tools to default permission levels
    Reset,
    /// Reset a single tool to default permission level
    ResetSingle { tool_name: String },
}

impl ToolsSubcommand {
    pub async fn execute(self, session: &mut ChatSession) -> Result<ChatState, ChatError> {
        let existing_tools: HashSet<&String> = session
            .conversation
            .tools
            .values()
            .flatten()
            .map(|FigTool::ToolSpecification(spec)| &spec.name)
            .collect();

        match self {
            Self::Schema => {
                let schema_json = serde_json::to_string_pretty(&session.conversation.tool_manager.schema)
                    .map_err(|e| ChatError::Custom(format!("Error converting tool schema to string: {e}").into()))?;
                queue!(session.stderr, style::Print(schema_json), style::Print("\n"))?;
            },
            Self::Trust { tool_names } => {
                let (valid_tools, invalid_tools): (Vec<String>, Vec<String>) = tool_names
                    .into_iter()
                    .partition(|tool_name| existing_tools.contains(tool_name));

                if !invalid_tools.is_empty() {
                    queue!(
                        session.stderr,
                        style::SetForegroundColor(Color::Red),
                        style::Print(format!("\nCannot trust '{}', ", invalid_tools.join("', '"))),
                        if invalid_tools.len() > 1 {
                            style::Print("they do not exist.")
                        } else {
                            style::Print("it does not exist.")
                        },
                        style::SetForegroundColor(Color::Reset),
                    )?;
                }
                if !valid_tools.is_empty() {
                    valid_tools.iter().for_each(|t| session.tool_permissions.trust_tool(t));
                    queue!(
                        session.stderr,
                        style::SetForegroundColor(Color::Green),
                        if valid_tools.len() > 1 {
                            style::Print(format!("\nTools '{}' are ", valid_tools.join("', '")))
                        } else {
                            style::Print(format!("\nTool '{}' is ", valid_tools[0]))
                        },
                        style::Print("now trusted. I will "),
                        style::SetAttribute(Attribute::Bold),
                        style::Print("not"),
                        style::SetAttribute(Attribute::Reset),
                        style::SetForegroundColor(Color::Green),
                        style::Print(format!(
                            " ask for confirmation before running {}.",
                            if valid_tools.len() > 1 {
                                "these tools"
                            } else {
                                "this tool"
                            }
                        )),
                        style::SetForegroundColor(Color::Reset),
                    )?;
                }
            },
            Self::Untrust { tool_names } => {
                let (valid_tools, invalid_tools): (Vec<String>, Vec<String>) = tool_names
                    .into_iter()
                    .partition(|tool_name| existing_tools.contains(tool_name));

                if !invalid_tools.is_empty() {
                    queue!(
                        session.stderr,
                        style::SetForegroundColor(Color::Red),
                        style::Print(format!("\nCannot untrust '{}', ", invalid_tools.join("', '"))),
                        if invalid_tools.len() > 1 {
                            style::Print("they do not exist.")
                        } else {
                            style::Print("it does not exist.")
                        },
                        style::SetForegroundColor(Color::Reset),
                    )?;
                }
                if !valid_tools.is_empty() {
                    valid_tools
                        .iter()
                        .for_each(|t| session.tool_permissions.untrust_tool(t));
                    queue!(
                        session.stderr,
                        style::SetForegroundColor(Color::Green),
                        if valid_tools.len() > 1 {
                            style::Print(format!("\nTools '{}' are ", valid_tools.join("', '")))
                        } else {
                            style::Print(format!("\nTool '{}' is ", valid_tools[0]))
                        },
                        style::Print("set to per-request confirmation."),
                        style::SetForegroundColor(Color::Reset),
                    )?;
                }
            },
            Self::TrustAll => {
                session
                    .conversation
                    .tools
                    .values()
                    .flatten()
                    .for_each(|FigTool::ToolSpecification(spec)| {
                        session.tool_permissions.trust_tool(spec.name.as_str());
                    });
                queue!(session.stderr, style::Print(TRUST_ALL_TEXT),)?;
            },
            Self::Reset => {
                session.tool_permissions.reset();
                queue!(
                    session.stderr,
                    style::SetForegroundColor(Color::Green),
                    style::Print("\nReset all tools to the default permission levels."),
                    style::SetForegroundColor(Color::Reset),
                )?;
            },
            Self::ResetSingle { tool_name } => {
                if session.tool_permissions.has(&tool_name) || session.tool_permissions.trust_all {
                    session.tool_permissions.reset_tool(&tool_name);
                    queue!(
                        session.stderr,
                        style::SetForegroundColor(Color::Green),
                        style::Print(format!("\nReset tool '{}' to the default permission level.", tool_name)),
                        style::SetForegroundColor(Color::Reset),
                    )?;
                } else {
                    queue!(
                        session.stderr,
                        style::SetForegroundColor(Color::Red),
                        style::Print(format!(
                            "\nTool '{}' does not exist or is already in default settings.",
                            tool_name
                        )),
                        style::SetForegroundColor(Color::Reset),
                    )?;
                }
            },
        };

        session.stderr.flush()?;

        Ok(ChatState::PromptUser {
            skip_printing_tools: true,
        })
    }
}
