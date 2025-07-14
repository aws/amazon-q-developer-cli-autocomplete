use clap::Args;
use crossterm::style::{
    Attribute,
    Color,
};
use crossterm::{
    execute,
    queue,
    style,
};

use crate::cli::chat::consts::CONTEXT_WINDOW_SIZE;
use crate::cli::chat::token_counter::{
    CharCount,
    TokenCount,
};
use crate::cli::chat::{
    ChatError,
    ChatSession,
    ChatState,
};
use crate::os::Os;
#[deny(missing_docs)]
#[derive(Debug, PartialEq, Args)]
pub struct UsageArgs;

impl UsageArgs {
    pub async fn execute(self, os: &Os, session: &mut ChatSession) -> Result<ChatState, ChatError> {
        let state = session
            .conversation
            .backend_conversation_state(os, true, &mut session.stderr)
            .await?;

        if !state.dropped_context_files.is_empty() {
            execute!(
                session.stderr,
                style::SetForegroundColor(Color::DarkYellow),
                style::Print("\nSome context files are dropped due to size limit, please run "),
                style::SetForegroundColor(Color::DarkGreen),
                style::Print("/context show "),
                style::SetForegroundColor(Color::DarkYellow),
                style::Print("to learn more.\n"),
                style::SetForegroundColor(style::Color::Reset)
            )?;
        }

        let data = state.calculate_conversation_size();
        let tool_specs_json: String = state
            .tools
            .values()
            .filter_map(|s| serde_json::to_string(s).ok())
            .collect::<Vec<String>>()
            .join("");
        let context_token_count: TokenCount = data.context_messages.into();
        let assistant_token_count: TokenCount = data.assistant_messages.into();
        let user_token_count: TokenCount = data.user_messages.into();
        let tools_char_count: CharCount = tool_specs_json.len().into(); // usize → CharCount
        let tools_token_count: TokenCount = tools_char_count.into(); // CharCount → TokenCount
        let total_token_used: TokenCount =
            (data.context_messages + data.user_messages + data.assistant_messages + tools_char_count).into();
        let window_width = session.terminal_width();
        // set a max width for the progress bar for better aesthetic
        let progress_bar_width = std::cmp::min(window_width, 80);

        let context_width =
            ((context_token_count.value() as f64 / CONTEXT_WINDOW_SIZE as f64) * progress_bar_width as f64) as usize;
        let assistant_width =
            ((assistant_token_count.value() as f64 / CONTEXT_WINDOW_SIZE as f64) * progress_bar_width as f64) as usize;
        let tools_width =
            ((tools_token_count.value() as f64 / CONTEXT_WINDOW_SIZE as f64) * progress_bar_width as f64) as usize;
        let user_width =
            ((user_token_count.value() as f64 / CONTEXT_WINDOW_SIZE as f64) * progress_bar_width as f64) as usize;

        let left_over_width = progress_bar_width
            - std::cmp::min(
                context_width + assistant_width + user_width + tools_width,
                progress_bar_width,
            );

        let is_overflow = (context_width + assistant_width + user_width + tools_width) > progress_bar_width;

        if is_overflow {
            queue!(
                session.stderr,
                style::Print(format!(
                    "\nCurrent context window ({} of {}k tokens used)\n",
                    total_token_used,
                    CONTEXT_WINDOW_SIZE / 1000
                )),
                style::SetForegroundColor(Color::DarkRed),
                style::Print("█".repeat(progress_bar_width)),
                style::SetForegroundColor(Color::Reset),
                style::Print(" "),
                style::Print(format!(
                    "{:.2}%",
                    (total_token_used.value() as f32 / CONTEXT_WINDOW_SIZE as f32) * 100.0
                )),
            )?;
        } else {
            queue!(
                session.stderr,
                style::Print(format!(
                    "\nCurrent context window ({} of {}k tokens used)\n",
                    total_token_used,
                    CONTEXT_WINDOW_SIZE / 1000
                )),
                // Context files
                style::SetForegroundColor(Color::DarkCyan),
                // add a nice visual to mimic "tiny" progress, so the overral progress bar doesn't look too
                // empty
                style::Print("|".repeat(if context_width == 0 && *context_token_count > 0 {
                    1
                } else {
                    0
                })),
                style::Print("█".repeat(context_width)),
                // Tools
                style::SetForegroundColor(Color::DarkRed),
                style::Print("|".repeat(if tools_width == 0 && *tools_token_count > 0 {
                    1
                } else {
                    0
                })),
                style::Print("█".repeat(tools_width)),
                // Assistant responses
                style::SetForegroundColor(Color::Blue),
                style::Print("|".repeat(if assistant_width == 0 && *assistant_token_count > 0 {
                    1
                } else {
                    0
                })),
                style::Print("█".repeat(assistant_width)),
                // User prompts
                style::SetForegroundColor(Color::Magenta),
                style::Print("|".repeat(if user_width == 0 && *user_token_count > 0 { 1 } else { 0 })),
                style::Print("█".repeat(user_width)),
                style::SetForegroundColor(Color::DarkGrey),
                style::Print("█".repeat(left_over_width)),
                style::Print(" "),
                style::SetForegroundColor(Color::Reset),
                style::Print(format!(
                    "{:.2}%",
                    (total_token_used.value() as f32 / CONTEXT_WINDOW_SIZE as f32) * 100.0
                )),
            )?;
        }

        execute!(session.stderr, style::Print("\n\n"))?;

        queue!(
            session.stderr,
            style::SetForegroundColor(Color::DarkCyan),
            style::Print("█ Context files: "),
            style::SetForegroundColor(Color::Reset),
            style::Print(format!(
                "~{} tokens ({:.2}%)\n",
                context_token_count,
                (context_token_count.value() as f32 / CONTEXT_WINDOW_SIZE as f32) * 100.0
            )),
            style::SetForegroundColor(Color::DarkRed),
            style::Print("█ Tools:    "),
            style::SetForegroundColor(Color::Reset),
            style::Print(format!(
                " ~{} tokens ({:.2}%)\n",
                tools_token_count,
                (tools_token_count.value() as f32 / CONTEXT_WINDOW_SIZE as f32) * 100.0
            )),
            style::SetForegroundColor(Color::Blue),
            style::Print("█ Q responses: "),
            style::SetForegroundColor(Color::Reset),
            style::Print(format!(
                "  ~{} tokens ({:.2}%)\n",
                assistant_token_count,
                (assistant_token_count.value() as f32 / CONTEXT_WINDOW_SIZE as f32) * 100.0
            )),
            style::SetForegroundColor(Color::Magenta),
            style::Print("█ Your prompts: "),
            style::SetForegroundColor(Color::Reset),
            style::Print(format!(
                " ~{} tokens ({:.2}%)\n\n",
                user_token_count,
                (user_token_count.value() as f32 / CONTEXT_WINDOW_SIZE as f32) * 100.0
            )),
        )?;

        match os.client.get_usage_limits().await {
            Ok(usage_response) => {
                if let Some(query_limit) = usage_response.limits().first() {
                    let total_limit = query_limit.value();
                    let percent_used = query_limit.percent_used().unwrap_or(0.0);

                    // Mock data
                    let queries_used = ((total_limit as f64 * percent_used / 100.0) as i64).max(1234);
                    let average_queries_per_day = 59.36;

                    // calculate reset date
                    let reset_date = (chrono::Local::now()
                        + chrono::Duration::days(usage_response.days_until_reset() as i64))
                    .format("%m/%d/%Y at %H:%M:%S");

                    queue!(
                        session.stderr,
                        style::Print("\n"),
                        style::SetAttribute(Attribute::Bold),
                        style::Print("📊 Usage limits\n"),
                        style::SetAttribute(Attribute::Reset),
                        // queries used
                        style::Print("• "),
                        style::SetForegroundColor(Color::Red),
                        style::Print(format!("{}", queries_used)),
                        style::SetForegroundColor(Color::Reset),
                        style::Print(" of "),
                        style::SetForegroundColor(Color::Red),
                        style::Print(format!("{}", total_limit)),
                        style::SetForegroundColor(Color::Reset),
                        style::Print(" queries used\n"),
                        // overages charged
                        style::Print("• $"),
                        style::SetForegroundColor(Color::Red),
                        style::Print(format!("{:.2}", average_queries_per_day)),
                        style::SetForegroundColor(Color::Reset),
                        style::Print(" incurred in average\n"),
                        // limit rest date
                        // Line 3
                        style::Print("• Limits reset on "),
                        style::SetForegroundColor(Color::Red),
                        style::Print(format!("{}\n", reset_date)),
                        style::SetForegroundColor(Color::Reset),
                    )?;
                }
            },
            Err(err) => {
                queue!(
                    session.stderr,
                    style::SetForegroundColor(Color::Red),
                    style::Print(format!("Failed to get usage limit: {}\n\n", err)),
                    style::SetForegroundColor(Color::Reset),
                )?;
            },
        }

        queue!(
            session.stderr,
            style::SetAttribute(Attribute::Bold),
            style::Print("\n💡 Pro Tips:\n"),
            style::SetAttribute(Attribute::Reset),
            style::SetForegroundColor(Color::DarkGrey),
            style::Print("Run "),
            style::SetForegroundColor(Color::DarkGreen),
            style::Print("/compact"),
            style::SetForegroundColor(Color::DarkGrey),
            style::Print(" to replace the conversation history with its summary\n"),
            style::Print("Run "),
            style::SetForegroundColor(Color::DarkGreen),
            style::Print("/clear"),
            style::SetForegroundColor(Color::DarkGrey),
            style::Print(" to erase the entire chat history\n"),
            style::Print("Run "),
            style::SetForegroundColor(Color::DarkGreen),
            style::Print("/context show"),
            style::SetForegroundColor(Color::DarkGrey),
            style::Print(" to see tokens per context file\n\n"),
            style::SetForegroundColor(Color::Reset),
        )?;

        Ok(ChatState::PromptUser {
            skip_printing_tools: true,
        })
    }
}
