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
use crate::cli::chat::token_counter::TokenCount;
use crate::cli::chat::{
    ChatError,
    ChatSession,
    ChatState,
};
use crate::platform::Context;

#[deny(missing_docs)]
#[derive(Debug, PartialEq, Args)]
pub struct UsageArgs;

impl UsageArgs {
    pub async fn execute(self, ctx: &Context, session: &mut ChatSession) -> Result<ChatState, ChatError> {
        let state = session
            .conversation
            .backend_conversation_state(ctx, true, &mut session.output)
            .await?;

        if !state.dropped_context_files.is_empty() {
            execute!(
                session.output,
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

        let context_token_count: TokenCount = data.context_messages.into();
        let assistant_token_count: TokenCount = data.assistant_messages.into();
        let user_token_count: TokenCount = data.user_messages.into();
        let total_token_used: TokenCount =
            (data.context_messages + data.user_messages + data.assistant_messages).into();

        let window_width = session.terminal_width();
        // set a max width for the progress bar for better aesthetic
        let progress_bar_width = std::cmp::min(window_width, 80);

        let context_width =
            ((context_token_count.value() as f64 / CONTEXT_WINDOW_SIZE as f64) * progress_bar_width as f64) as usize;
        let assistant_width =
            ((assistant_token_count.value() as f64 / CONTEXT_WINDOW_SIZE as f64) * progress_bar_width as f64) as usize;
        let user_width =
            ((user_token_count.value() as f64 / CONTEXT_WINDOW_SIZE as f64) * progress_bar_width as f64) as usize;

        let left_over_width =
            progress_bar_width - std::cmp::min(context_width + assistant_width + user_width, progress_bar_width);

        let is_overflow = (context_width + assistant_width + user_width) > progress_bar_width;

        if is_overflow {
            queue!(
                session.output,
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
                session.output,
                style::Print(format!(
                    "\nCurrent context window ({} of {}k tokens used)\n",
                    total_token_used,
                    CONTEXT_WINDOW_SIZE / 1000
                )),
                style::SetForegroundColor(Color::DarkCyan),
                // add a nice visual to mimic "tiny" progress, so the overral progress bar doesn't look too
                // empty
                style::Print("|".repeat(if context_width == 0 && *context_token_count > 0 {
                    1
                } else {
                    0
                })),
                style::Print("█".repeat(context_width)),
                style::SetForegroundColor(Color::Blue),
                style::Print("|".repeat(if assistant_width == 0 && *assistant_token_count > 0 {
                    1
                } else {
                    0
                })),
                style::Print("█".repeat(assistant_width)),
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

        execute!(session.output, style::Print("\n\n"))?;

        queue!(
            session.output,
            style::SetForegroundColor(Color::DarkCyan),
            style::Print("█ Context files: "),
            style::SetForegroundColor(Color::Reset),
            style::Print(format!(
                "~{} tokens ({:.2}%)\n",
                context_token_count,
                (context_token_count.value() as f32 / CONTEXT_WINDOW_SIZE as f32) * 100.0
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

        queue!(
            session.output,
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
