use clap::Args;
use crossterm::style::Color;
use crossterm::{
    execute,
    style,
};
use eyre::Result;
use regex::Regex;
use spinners::{
    Spinner,
    Spinners,
};

use crate::cli::chat::parser::{
    ResponseEvent,
    ResponseParser,
};
use crate::cli::chat::{
    ChatError,
    ChatSession,
    ChatState,
};
use crate::os::Os;

/// Arguments for the complete command
#[derive(Debug, Args, PartialEq)]
pub struct CompleteArgs {
    /// Custom context or instruction for the completion
    #[arg(long, short = 'c')]
    context: Option<String>,

    /// Show the generated completions without sending
    #[arg(long)]
    preview: bool,

    /// Number of completion options to generate (1-5)
    #[arg(long, short = 'n', default_value = "3")]
    count: u8,
}

impl CompleteArgs {
    pub async fn execute(self, os: &mut Os, session: &mut ChatSession) -> Result<ChatState, ChatError> {
        // Validate count
        let count = self.count.clamp(1, 5);

        // Check if we have conversation history
        if session.conversation.history().is_empty() {
            execute!(
                session.stderr,
                style::SetForegroundColor(Color::Yellow),
                style::Print("No conversation history available. Start a conversation first.\n"),
                style::SetForegroundColor(Color::Reset)
            )?;
            return Ok(ChatState::PromptUser {
                skip_printing_tools: true,
            });
        }

        // Create completion request
        let completion_request = session
            .conversation
            .create_completion_request(os, self.context.as_ref(), count)
            .await?;

        // Show spinner while generating completions
        let spinner = Spinner::new(Spinners::Dots, "Generating completions...".to_string());

        // Send request to LLM
        let response = os.client.send_message(completion_request).await?;

        // Parse the response
        let completions = parse_completions_response(response).await?;

        // Stop spinner
        drop(spinner);
        execute!(
            session.stderr,
            crossterm::terminal::Clear(crossterm::terminal::ClearType::CurrentLine),
            crossterm::cursor::MoveToColumn(0)
        )?;

        if completions.is_empty() {
            execute!(
                session.stderr,
                style::SetForegroundColor(Color::Yellow),
                style::Print("No completions could be generated.\n"),
                style::SetForegroundColor(Color::Reset)
            )?;
            return Ok(ChatState::PromptUser {
                skip_printing_tools: true,
            });
        }

        if self.preview {
            display_completions(&completions, session)?;
            Ok(ChatState::PromptUser {
                skip_printing_tools: true,
            })
        } else {
            select_and_send_completion(completions, session).await
        }
    }
}

async fn select_and_send_completion(
    completions: Vec<String>,
    session: &mut ChatSession,
) -> Result<ChatState, ChatError> {
    // Display completions with selection prompt
    execute!(
        session.stderr,
        style::SetForegroundColor(Color::Cyan),
        style::Print("Select a completion to send:\n\n"),
        style::SetForegroundColor(Color::Reset)
    )?;

    for (i, completion) in completions.iter().enumerate() {
        execute!(
            session.stderr,
            style::SetForegroundColor(Color::Green),
            style::Print(format!("  {}. ", i + 1)),
            style::SetForegroundColor(Color::Reset),
            style::Print(format!("{}\n", completion))
        )?;
    }

    execute!(
        session.stderr,
        style::Print(format!(
            "\nEnter selection (1-{}), or press Enter to cancel: ",
            completions.len()
        ))
    )?;

    // Read user selection using the session's method
    let input = session.read_user_input("", true);

    match input {
        Some(selection) if !selection.trim().is_empty() => {
            if let Ok(index) = selection.trim().parse::<usize>() {
                if index > 0 && index <= completions.len() {
                    let selected_completion = completions[index - 1].clone();

                    // // Display the selected completion
                    // execute!(
                    //     session.stderr,
                    //     style::SetForegroundColor(Color::Green),
                    //     style::Print(format!("Sending: {}\n\n", selected_completion)),
                    //     style::SetForegroundColor(Color::Reset)
                    // )?;

                    // Send the completion as user input
                    return Ok(ChatState::HandleInput {
                        input: selected_completion,
                    });
                }
            }

            execute!(
                session.stderr,
                style::SetForegroundColor(Color::Red),
                style::Print("Invalid selection.\n"),
                style::SetForegroundColor(Color::Reset)
            )?;
        },
        _ => {
            execute!(session.stderr, style::Print("Completion cancelled.\n"))?;
        },
    }

    Ok(ChatState::PromptUser {
        skip_printing_tools: true,
    })
}

async fn parse_completions_response(
    response: crate::api_client::send_message_output::SendMessageOutput,
) -> Result<Vec<String>, ChatError> {
    let mut parser = ResponseParser::new(response);
    let mut full_response = String::new();

    loop {
        match parser.recv().await {
            Ok(ResponseEvent::AssistantText(text)) => {
                full_response.push_str(&text);
            },
            Ok(ResponseEvent::EndStream { .. }) => break,
            Ok(_) => {}, // Ignore other events
            Err(err) => {
                return Err(ChatError::Custom(
                    format!("Failed to parse completion response: {}", err).into(),
                ));
            },
        }
    }

    // Parse numbered list from response
    extract_completions_from_text(&full_response)
}

fn extract_completions_from_text(text: &str) -> Result<Vec<String>, ChatError> {
    let mut completions = Vec::new();

    for line in text.lines() {
        let line = line.trim();

        // Look for numbered list items (1. 2. 3. etc.)
        if let Some(completion) = extract_numbered_item(line) {
            if !completion.is_empty() {
                completions.push(completion);
            }
        }
    }

    // If no numbered items found, try to split by common delimiters
    if completions.is_empty() {
        completions = text
            .split('\n')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty() && !s.starts_with('[') && !s.starts_with("SYSTEM"))
            .take(5)
            .map(|s| s.to_string())
            .collect();
    }

    Ok(completions)
}

fn display_completions(completions: &[String], session: &mut ChatSession) -> Result<(), ChatError> {
    execute!(
        session.stderr,
        style::SetForegroundColor(Color::Cyan),
        style::Print("Generated completions:\n\n"),
        style::SetForegroundColor(Color::Reset)
    )?;

    for (i, completion) in completions.iter().enumerate() {
        execute!(
            session.stderr,
            style::SetForegroundColor(Color::Green),
            style::Print(format!("  {}. ", i + 1)),
            style::SetForegroundColor(Color::Reset),
            style::Print(format!("{}\n", completion))
        )?;
    }

    execute!(session.stderr, style::Print("\n"))?;
    Ok(())
}

fn extract_numbered_item(line: &str) -> Option<String> {
    // Match patterns like "1. Text", "2) Text", "• Text", "- Text"
    let patterns = [
        Regex::new(r"^\d+\.\s*(.+)$").ok()?,
        Regex::new(r"^\d+\)\s*(.+)$").ok()?,
        Regex::new(r"^[•\-\*]\s*(.+)$").ok()?,
    ];

    for pattern in &patterns {
        if let Some(captures) = pattern.captures(line) {
            if let Some(content) = captures.get(1) {
                return Some(content.as_str().trim().to_string());
            }
        }
    }

    None
}
