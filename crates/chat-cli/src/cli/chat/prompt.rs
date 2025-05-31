use std::borrow::Cow;

use eyre::Result;
use rustyline::completion::{
    Completer,
    FilenameCompleter,
    extract_word,
};
use rustyline::error::ReadlineError;
use rustyline::highlight::{
    CmdKind,
    Highlighter,
};
use rustyline::history::DefaultHistory;
use rustyline::validate::{
    ValidationContext,
    ValidationResult,
    Validator,
};
use rustyline::{
    Cmd,
    Completer,
    CompletionType,
    Config,
    Context,
    EditMode,
    Editor,
    EventHandler,
    Helper,
    Hinter,
    KeyCode,
    KeyEvent,
    Modifiers,
};
use winnow::stream::AsChar;

use crate::database::Database;
use crate::database::settings::Setting;

pub const COMMANDS: &[&str] = &[
    "/clear",
    "/help",
    "/editor",
    "/issue",
    // "/acceptall", /// Functional, but deprecated in favor of /tools trustall
    "/quit",
    "/tools",
    "/tools trust",
    "/tools untrust",
    "/tools trustall",
    "/tools reset",
    "/profile",
    "/profile help",
    "/profile list",
    "/profile create",
    "/profile delete",
    "/profile rename",
    "/profile set",
    "/context help",
    "/context show",
    "/context show --expand",
    "/context add",
    "/context add --global",
    "/context rm",
    "/context rm --global",
    "/context clear",
    "/context clear --global",
    "/context hooks help",
    "/context hooks add",
    "/context hooks rm",
    "/context hooks enable",
    "/context hooks disable",
    "/context hooks enable-all",
    "/context hooks disable-all",
    "/compact",
    "/compact help",
    "/usage",
    "/save",
    "/load",
];

/// Components extracted from a prompt string
#[derive(Debug)]
struct PromptComponents {
    profile: Option<String>,
    warning: bool,
}

/// Parse prompt components from a plain text prompt
fn parse_prompt_components(prompt: &str) -> Option<PromptComponents> {
    // Expected format: "[profile] !> " or "> " or "!> " etc.
    let mut profile = None;
    let mut warning = false;
    let mut remaining = prompt.trim();

    // Check for profile pattern [profile]
    if let Some(start) = remaining.find('[') {
        if let Some(end) = remaining.find(']') {
            if start < end {
                profile = Some(remaining[start + 1..end].to_string());
                remaining = remaining[end + 1..].trim_start();
            }
        }
    }

    // Check for warning symbol !
    if remaining.starts_with('!') {
        warning = true;
        remaining = remaining[1..].trim_start();
    }

    // Should end with "> "
    if remaining.trim_end() == ">" {
        Some(PromptComponents { profile, warning })
    } else {
        None
    }
}

pub fn generate_prompt(current_profile: Option<&str>, warning: bool) -> String {
    // Generate plain text prompt that will be colored by highlight_prompt
    let warning_symbol = if warning { "!" } else { "" };
    let profile_part = current_profile
        .filter(|&p| p != "default")
        .map(|p| format!("[{p}] "))
        .unwrap_or_default();

    format!("{profile_part}{warning_symbol}> ")
}

/// Complete commands that start with a slash
fn complete_command(word: &str, start: usize) -> (usize, Vec<String>) {
    (
        start,
        COMMANDS
            .iter()
            .filter(|p| p.starts_with(word))
            .map(|s| (*s).to_owned())
            .collect(),
    )
}

/// A wrapper around FilenameCompleter that provides enhanced path detection
/// and completion capabilities for the chat interface.
pub struct PathCompleter {
    /// The underlying filename completer from rustyline
    filename_completer: FilenameCompleter,
}

impl PathCompleter {
    /// Creates a new PathCompleter instance
    pub fn new() -> Self {
        Self {
            filename_completer: FilenameCompleter::new(),
        }
    }

    /// Attempts to complete a file path at the given position in the line
    pub fn complete_path(
        &self,
        line: &str,
        pos: usize,
        ctx: &Context<'_>,
    ) -> Result<(usize, Vec<String>), ReadlineError> {
        // Use the filename completer to get path completions
        match self.filename_completer.complete(line, pos, ctx) {
            Ok((pos, completions)) => {
                // Convert the filename completer's pairs to strings
                let file_completions: Vec<String> = completions.iter().map(|pair| pair.replacement.clone()).collect();

                // Return the completions if we have any
                Ok((pos, file_completions))
            },
            Err(err) => Err(err),
        }
    }
}

pub struct PromptCompleter {
    sender: std::sync::mpsc::Sender<Option<String>>,
    receiver: std::sync::mpsc::Receiver<Vec<String>>,
}

impl PromptCompleter {
    fn new(sender: std::sync::mpsc::Sender<Option<String>>, receiver: std::sync::mpsc::Receiver<Vec<String>>) -> Self {
        PromptCompleter { sender, receiver }
    }

    fn complete_prompt(&self, word: &str) -> Result<Vec<String>, ReadlineError> {
        let sender = &self.sender;
        let receiver = &self.receiver;
        sender
            .send(if !word.is_empty() { Some(word.to_string()) } else { None })
            .map_err(|e| ReadlineError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))?;
        let prompt_info = receiver
            .recv()
            .map_err(|e| ReadlineError::Io(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))?
            .iter()
            .map(|n| format!("@{n}"))
            .collect::<Vec<_>>();

        Ok(prompt_info)
    }
}

pub struct ChatCompleter {
    path_completer: PathCompleter,
    prompt_completer: PromptCompleter,
}

impl ChatCompleter {
    fn new(sender: std::sync::mpsc::Sender<Option<String>>, receiver: std::sync::mpsc::Receiver<Vec<String>>) -> Self {
        Self {
            path_completer: PathCompleter::new(),
            prompt_completer: PromptCompleter::new(sender, receiver),
        }
    }
}

impl Completer for ChatCompleter {
    type Candidate = String;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> Result<(usize, Vec<Self::Candidate>), ReadlineError> {
        let (start, word) = extract_word(line, pos, None, |c| c.is_space());

        // Handle command completion
        if word.starts_with('/') {
            return Ok(complete_command(word, start));
        }

        if line.starts_with('@') {
            let search_word = line.strip_prefix('@').unwrap_or("");
            if let Ok(completions) = self.prompt_completer.complete_prompt(search_word) {
                if !completions.is_empty() {
                    return Ok((0, completions));
                }
            }
        }

        // Handle file path completion as fallback
        if let Ok((pos, completions)) = self.path_completer.complete_path(line, pos, _ctx) {
            if !completions.is_empty() {
                return Ok((pos, completions));
            }
        }

        // Default: no completions
        Ok((start, Vec::new()))
    }
}

/// Custom validator for multi-line input
pub struct MultiLineValidator;

impl Validator for MultiLineValidator {
    fn validate(&self, ctx: &mut ValidationContext<'_>) -> rustyline::Result<ValidationResult> {
        let input = ctx.input();

        // Check for explicit multi-line markers
        if input.starts_with("```") && !input.ends_with("```") {
            return Ok(ValidationResult::Incomplete);
        }

        // Check for backslash continuation
        if input.ends_with('\\') {
            return Ok(ValidationResult::Incomplete);
        }

        Ok(ValidationResult::Valid(None))
    }
}

#[derive(Helper, Completer, Hinter)]
pub struct ChatHelper {
    #[rustyline(Completer)]
    completer: ChatCompleter,
    #[rustyline(Hinter)]
    hinter: (),
    validator: MultiLineValidator,
}

impl Validator for ChatHelper {
    fn validate(&self, ctx: &mut ValidationContext<'_>) -> rustyline::Result<ValidationResult> {
        self.validator.validate(ctx)
    }
}

impl Highlighter for ChatHelper {
    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        Cow::Owned(format!("\x1b[1m{hint}\x1b[m"))
    }

    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        Cow::Borrowed(line)
    }

    fn highlight_char(&self, _line: &str, _pos: usize, _kind: CmdKind) -> bool {
        false
    }

    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(&'s self, prompt: &'p str, _default: bool) -> Cow<'b, str> {
        // Parse the plain text prompt to extract profile and warning information
        // and apply colors using ANSI escape codes that rustyline can handle properly
        if let Some(captures) = parse_prompt_components(prompt) {
            let profile_part = if let Some(profile) = captures.profile {
                format!("\x1b[36m[{}] \x1b[0m", profile) // cyan for profile
            } else {
                String::new()
            };

            let warning_part = if captures.warning {
                "\x1b[31m!\x1b[0m".to_string() // red for warning
            } else {
                String::new()
            };

            let prompt_part = "\x1b[36;1m> \x1b[0m"; // cyan bold for prompt

            Cow::Owned(format!("{}{}{}", profile_part, warning_part, prompt_part))
        } else {
            // Fallback: return the prompt as-is
            Cow::Borrowed(prompt)
        }
    }
}

pub fn rl(
    database: &Database,
    sender: std::sync::mpsc::Sender<Option<String>>,
    receiver: std::sync::mpsc::Receiver<Vec<String>>,
) -> Result<Editor<ChatHelper, DefaultHistory>> {
    let edit_mode = match database.settings.get_string(Setting::ChatEditMode).as_deref() {
        Some("vi" | "vim") => EditMode::Vi,
        _ => EditMode::Emacs,
    };
    let config = Config::builder()
        .history_ignore_space(true)
        .completion_type(CompletionType::List)
        .edit_mode(edit_mode)
        .build();
    let h = ChatHelper {
        completer: ChatCompleter::new(sender, receiver),
        hinter: (),
        validator: MultiLineValidator,
    };
    let mut rl = Editor::with_config(config)?;
    rl.set_helper(Some(h));

    // Add custom keybinding for Alt+Enter to insert a newline
    rl.bind_sequence(
        KeyEvent(KeyCode::Enter, Modifiers::ALT),
        EventHandler::Simple(Cmd::Insert(1, "\n".to_string())),
    );

    // Add custom keybinding for Ctrl+J to insert a newline
    rl.bind_sequence(
        KeyEvent(KeyCode::Char('j'), Modifiers::CTRL),
        EventHandler::Simple(Cmd::Insert(1, "\n".to_string())),
    );

    Ok(rl)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_generate_prompt() {
        // Test default prompt (no profile)
        assert_eq!(generate_prompt(None, false), "> ");
        // Test default prompt with warning
        assert_eq!(generate_prompt(None, true), "!> ");
        // Test default profile (should be same as no profile)
        assert_eq!(generate_prompt(Some("default"), false), "> ");
        // Test custom profile
        assert_eq!(generate_prompt(Some("test-profile"), false), "[test-profile] > ");
        // Test another custom profile with warning
        assert_eq!(generate_prompt(Some("dev"), true), "[dev] !> ");
    }

    #[test]
    fn test_parse_prompt_components() {
        // Test basic prompt
        let components = parse_prompt_components("> ").unwrap();
        assert!(components.profile.is_none());
        assert!(!components.warning);

        // Test warning prompt
        let components = parse_prompt_components("!> ").unwrap();
        assert!(components.profile.is_none());
        assert!(components.warning);

        // Test profile prompt
        let components = parse_prompt_components("[test] > ").unwrap();
        assert_eq!(components.profile.as_deref(), Some("test"));
        assert!(!components.warning);

        // Test profile with warning
        let components = parse_prompt_components("[dev] !> ").unwrap();
        assert_eq!(components.profile.as_deref(), Some("dev"));
        assert!(components.warning);

        // Test invalid prompt
        assert!(parse_prompt_components("invalid").is_none());
    }

    #[test]
    fn test_chat_completer_command_completion() {
        let (prompt_request_sender, _) = std::sync::mpsc::channel::<Option<String>>();
        let (_, prompt_response_receiver) = std::sync::mpsc::channel::<Vec<String>>();
        let completer = ChatCompleter::new(prompt_request_sender, prompt_response_receiver);
        let line = "/h";
        let pos = 2; // Position at the end of "/h"

        // Create a mock context with empty history
        let empty_history = DefaultHistory::new();
        let ctx = Context::new(&empty_history);

        // Get completions
        let (start, completions) = completer.complete(line, pos, &ctx).unwrap();

        // Verify start position
        assert_eq!(start, 0);

        // Verify completions contain expected commands
        assert!(completions.contains(&"/help".to_string()));
    }

    #[test]
    fn test_chat_completer_no_completion() {
        let (prompt_request_sender, _) = std::sync::mpsc::channel::<Option<String>>();
        let (_, prompt_response_receiver) = std::sync::mpsc::channel::<Vec<String>>();
        let completer = ChatCompleter::new(prompt_request_sender, prompt_response_receiver);
        let line = "Hello, how are you?";
        let pos = line.len();

        // Create a mock context with empty history
        let empty_history = DefaultHistory::new();
        let ctx = Context::new(&empty_history);

        // Get completions
        let (_, completions) = completer.complete(line, pos, &ctx).unwrap();

        // Verify no completions are returned for regular text
        assert!(completions.is_empty());
    }
}
