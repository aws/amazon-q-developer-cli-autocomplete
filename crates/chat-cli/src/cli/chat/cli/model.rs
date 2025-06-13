use clap::Args;
use crossterm::style::{
    self,
    Color,
};
use crossterm::{
    execute,
    queue,
};
use dialoguer::Select;

use crate::cli::chat::{
    ChatError,
    ChatState,
};

pub struct ModelOption {
    pub name: &'static str,
    pub model_id: &'static str,
}

pub const MODEL_OPTIONS: [ModelOption; 3] = [
    ModelOption {
        name: "claude-4-sonnet",
        model_id: "CLAUDE_SONNET_4_20250514_V1_0",
    },
    ModelOption {
        name: "claude-3.7-sonnet",
        model_id: "CLAUDE_3_7_SONNET_20250219_V1_0",
    },
    ModelOption {
        name: "claude-3.5-sonnet",
        model_id: "CLAUDE_3_5_SONNET_20241022_V2_0",
    },
];

pub const DEFAULT_MODEL_ID: &str = "CLAUDE_3_7_SONNET_20250219_V1_0";

#[deny(missing_docs)]
#[derive(Debug, PartialEq, Args)]
pub struct ModelArgs;

impl ModelArgs {
    pub async fn execute(self) -> Result<ChatState, ChatError> {
        queue!(output, style::Print("\n"))?;
        let active_model_id = self.conversation.model.as_deref();
        let labels: Vec<String> = MODEL_OPTIONS
            .iter()
            .map(|opt| {
                if (opt.model_id.is_empty() && active_model_id.is_none()) || Some(opt.model_id) == active_model_id {
                    format!("{} (active)", opt.name)
                } else {
                    opt.name.to_owned()
                }
            })
            .collect();

        let selection: Option<_> = match Select::with_theme(&crate::util::dialoguer_theme())
            .with_prompt("Select a model for this chat session")
            .items(&labels)
            .default(0)
            .interact_on_opt(&dialoguer::console::Term::stdout())
        {
            Ok(sel) => {
                let _ = crossterm::execute!(
                    std::io::stdout(),
                    crossterm::style::SetForegroundColor(crossterm::style::Color::Magenta)
                );
                sel
            },
            // Ctrl‑C -> Err(Interrupted)
            Err(DError::IO(ref e)) if e.kind() == io::ErrorKind::Interrupted => None,
            Err(e) => return Err(ChatError::Custom(format!("Failed to choose model: {e}").into())),
        };

        queue!(output, style::ResetColor)?;

        if let Some(index) = selection {
            let selected = &MODEL_OPTIONS[index];
            let model_id_str = selected.model_id.to_string();
            self.conversation.model = Some(model_id_str);

            queue!(
                output,
                style::Print("\n"),
                style::Print(format!(" Using {}\n\n", selected.name)),
                style::ResetColor,
                style::SetForegroundColor(Color::Reset),
                style::SetBackgroundColor(Color::Reset),
            )?;
        }

        execute!(output, style::ResetColor)?;

        Ok(ChatState::PromptUser {
            tool_uses: None,
            pending_tool_index: None,
            skip_printing_tools: false,
        })
    }
}
