use std::io::Write;

use clap::Subcommand;
use crossterm::queue;
use crossterm::style::{
    self,
    Color,
};
use eyre::Result;
use semantic_search_client::{
    KnowledgeContext,
    OperationStatus,
    SystemStatus,
};

use crate::cli::chat::tools::sanitize_path_tool_arg;
use crate::cli::chat::{
    ChatError,
    ChatState,
};
use crate::database::Database;
use crate::database::settings::Setting;
use crate::platform::Context;
use crate::util::knowledge_store::KnowledgeStore;

/// Knowledge base management commands
#[derive(Clone, Debug, PartialEq, Eq, Subcommand)]
pub enum KnowledgeSubcommand {
    /// Display the knowledge base contents
    Show,
    /// Add a file or directory to knowledge base
    Add { path: String },
    /// Remove specified knowledge context by path
    #[command(alias = "rm")]
    Remove { path: String },
    /// Update a file or directory in knowledge base
    Update { path: String },
    /// Remove all knowledge contexts
    Clear,
    /// Show background operation status
    Status,
    /// Cancel a background operation
    Cancel {
        /// Operation ID to cancel (optional - cancels most recent if not provided)
        operation_id: Option<String>,
    },
    /// Show help for knowledge commands
    Help,
}

#[derive(Debug)]
enum OperationResult {
    Success(String),
    Info(String),
    Warning(String),
    Error(String),
}

impl KnowledgeSubcommand {
    pub async fn execute(
        self,
        ctx: &Context,
        database: &Database,
        output: &mut impl Write,
    ) -> Result<ChatState, ChatError> {
        if !Self::is_feature_enabled(database) {
            Self::write_feature_disabled_message(output)?;
            return Ok(Self::default_chat_state());
        }

        let result = self.execute_operation(ctx, output).await;

        Self::write_operation_result(output, result)?;

        Ok(Self::default_chat_state())
    }

    fn is_feature_enabled(database: &Database) -> bool {
        database.settings.get_bool(Setting::EnabledKnowledge).unwrap_or(false)
    }

    fn write_feature_disabled_message(output: &mut impl Write) -> Result<(), std::io::Error> {
        queue!(
            output,
            style::SetForegroundColor(Color::Red),
            style::Print("\nKnowledge tool is disabled. Enable it with: q settings chat.enableKnowledge true\n\n"),
            style::SetForegroundColor(Color::Reset)
        )
    }

    fn default_chat_state() -> ChatState {
        ChatState::PromptUser {
            tool_uses: None,
            pending_tool_index: None,
            skip_printing_tools: true,
        }
    }

    async fn execute_operation(&self, ctx: &Context, output: &mut impl Write) -> OperationResult {
        match self {
            KnowledgeSubcommand::Show => {
                match Self::handle_show(output).await {
                    Ok(_) => OperationResult::Info("".to_string()), // Empty Info, formatting already done
                    Err(e) => OperationResult::Error(format!("Failed to show contexts: {}", e)),
                }
            },
            KnowledgeSubcommand::Add { path } => Self::handle_add(ctx, path).await,
            KnowledgeSubcommand::Remove { path } => Self::handle_remove(ctx, path).await,
            KnowledgeSubcommand::Update { path } => Self::handle_update(ctx, path).await,
            KnowledgeSubcommand::Clear => Self::handle_clear(output).await,
            KnowledgeSubcommand::Status => Self::handle_status().await,
            KnowledgeSubcommand::Cancel { operation_id } => Self::handle_cancel(operation_id.as_deref()).await,
            KnowledgeSubcommand::Help => OperationResult::Success(Self::help_text()),
        }
    }

    async fn handle_show(output: &mut impl Write) -> Result<(), std::io::Error> {
        let async_knowledge_store = KnowledgeStore::get_async_instance().await;
        let store = async_knowledge_store.lock().await;

        // Use the async get_all method which is concurrent with indexing
        let contexts = store.get_all().await.unwrap_or_else(|e| {
            // Write error to output using queue system
            let _ = queue!(
                output,
                style::SetForegroundColor(Color::Red),
                style::Print(&format!("Error getting contexts: {}\n", e)),
                style::ResetColor
            );
            Vec::new()
        });

        Self::format_contexts(output, &contexts)
    }

    fn format_contexts(output: &mut impl Write, contexts: &[KnowledgeContext]) -> Result<(), std::io::Error> {
        if contexts.is_empty() {
            queue!(
                output,
                style::Print("\nNo knowledge base entries found.\n"),
                style::Print("💡 Tip: If indexing is in progress, contexts may not appear until indexing completes.\n"),
                style::Print("   Use 'knowledge status' to check active operations.\n\n")
            )?;
        } else {
            queue!(
                output,
                style::Print("\n📚 Knowledge Base Contexts:\n"),
                style::Print(format!("{}\n", "━".repeat(80)))
            )?;

            for context in contexts {
                Self::format_single_context(output, &context)?;
                queue!(output, style::Print(format!("{}\n", "━".repeat(80))))?;
            }
            // Add final newline to match original formatting exactly
            queue!(output, style::Print("\n"))?;
        }
        Ok(())
    }

    fn format_single_context(
        output: &mut (impl Write + Sized),
        context: &&KnowledgeContext,
    ) -> Result<(), std::io::Error> {
        queue!(
            output,
            style::SetAttribute(style::Attribute::Bold),
            style::SetForegroundColor(Color::Cyan),
            style::Print(format!("📂 {}: ", context.id)),
            style::SetForegroundColor(Color::Green),
            style::Print(&context.name),
            style::SetAttribute(style::Attribute::Reset),
            style::Print("\n")
        )?;

        queue!(
            output,
            style::Print(format!("   Description: {}\n", context.description)),
            style::Print(format!(
                "   Created: {}\n",
                context.created_at.format("%Y-%m-%d %H:%M:%S")
            )),
            style::Print(format!(
                "   Updated: {}\n",
                context.updated_at.format("%Y-%m-%d %H:%M:%S")
            ))
        )?;

        if let Some(path) = &context.source_path {
            queue!(output, style::Print(format!("   Source: {}\n", path)))?;
        }

        queue!(
            output,
            style::Print("   Items: "),
            style::SetForegroundColor(Color::Yellow),
            style::Print(format!("{}", context.item_count)),
            style::SetForegroundColor(Color::Reset),
            style::Print(" | Persistent: ")
        )?;

        if context.persistent {
            queue!(
                output,
                style::SetForegroundColor(Color::Green),
                style::Print("Yes"),
                style::SetForegroundColor(Color::Reset),
                style::Print("\n")
            )?;
        } else {
            queue!(
                output,
                style::SetForegroundColor(Color::Yellow),
                style::Print("No"),
                style::SetForegroundColor(Color::Reset),
                style::Print("\n")
            )?;
        }
        Ok(())
    }

    /// Handle add operation
    async fn handle_add(ctx: &Context, path: &str) -> OperationResult {
        match Self::validate_and_sanitize_path(ctx, path) {
            Ok(sanitized_path) => {
                let async_knowledge_store = KnowledgeStore::get_async_instance().await;
                let mut store = async_knowledge_store.lock().await;

                // Use the async add method which is fire-and-forget
                match store.add(path, &sanitized_path).await {
                    Ok(message) => OperationResult::Info(message),
                    Err(e) => OperationResult::Error(format!("Failed to add to knowledge base: {}", e)),
                }
            },
            Err(e) => OperationResult::Error(e),
        }
    }

    /// Handle remove operation
    async fn handle_remove(ctx: &Context, path: &str) -> OperationResult {
        let sanitized_path = sanitize_path_tool_arg(ctx, path);
        let async_knowledge_store = KnowledgeStore::get_async_instance().await;
        let mut store = async_knowledge_store.lock().await;

        // Try path first, then name
        if store.remove_by_path(&sanitized_path.to_string_lossy()).await.is_ok() {
            OperationResult::Success(format!("Removed context with path '{}'", path))
        } else if store.remove_by_name(path).await.is_ok() {
            OperationResult::Success(format!("Removed context with name '{}'", path))
        } else {
            OperationResult::Warning(format!("Entry not found in knowledge base: {}", path))
        }
    }

    /// Handle update operation
    async fn handle_update(ctx: &Context, path: &str) -> OperationResult {
        match Self::validate_and_sanitize_path(ctx, path) {
            Ok(sanitized_path) => {
                let async_knowledge_store = KnowledgeStore::get_async_instance().await;
                let mut store = async_knowledge_store.lock().await;

                match store.update_by_path(&sanitized_path).await {
                    Ok(message) => OperationResult::Info(message),
                    Err(e) => OperationResult::Error(format!("Failed to update: {}", e)),
                }
            },
            Err(e) => OperationResult::Error(e),
        }
    }

    /// Handle clear operation
    async fn handle_clear(output: &mut impl Write) -> OperationResult {
        // Require confirmation
        queue!(
            output,
            style::Print("⚠️  This will remove ALL knowledge base entries. Are you sure? (y/N): ")
        )
        .unwrap();
        output.flush().unwrap();

        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_err() {
            return OperationResult::Error("Failed to read input".to_string());
        }

        let input = input.trim().to_lowercase();
        if input != "y" && input != "yes" {
            return OperationResult::Info("Clear operation cancelled".to_string());
        }

        let async_knowledge_store = KnowledgeStore::get_async_instance().await;
        let mut store = async_knowledge_store.lock().await;

        // First, cancel any pending operations
        queue!(output, style::Print("🛑 Cancelling any pending operations...\n")).unwrap();
        if let Err(e) = store.cancel_operation(None).await {
            queue!(
                output,
                style::Print(&format!("⚠️  Warning: Failed to cancel operations: {}\n", e))
            )
            .unwrap();
        }

        // Now perform immediate synchronous clear
        queue!(output, style::Print("🗑️  Clearing all knowledge base entries...\n")).unwrap();
        match store.clear_immediate().await {
            Ok(message) => OperationResult::Success(message),
            Err(e) => OperationResult::Error(format!("Failed to clear: {}", e)),
        }
    }

    /// Handle status operation
    async fn handle_status() -> OperationResult {
        let async_knowledge_store = KnowledgeStore::get_async_instance().await;
        let store = async_knowledge_store.lock().await;

        match store.get_status_data().await {
            Ok(status_data) => {
                let formatted_status = Self::format_status_display(&status_data);
                OperationResult::Info(formatted_status)
            },
            Err(e) => OperationResult::Error(format!("Failed to get status: {}", e)),
        }
    }

    /// Format status data for display (UI rendering responsibility)
    fn format_status_display(status: &SystemStatus) -> String {
        let mut status_lines = Vec::new();

        // Show context summary
        status_lines.push(format!(
            "📚 Total contexts: {} ({} persistent, {} volatile)",
            status.total_contexts, status.persistent_contexts, status.volatile_contexts
        ));

        if status.operations.is_empty() {
            status_lines.push("✅ No active operations".to_string());
            return status_lines.join("\n");
        }

        status_lines.push("📊 Active Operations:".to_string());
        status_lines.push(format!(
            "  📈 Queue Status: {} active, {} waiting (max {} concurrent)",
            status.active_count, status.waiting_count, status.max_concurrent
        ));

        for op in &status.operations {
            let formatted_operation = Self::format_operation_display(op);
            status_lines.push(formatted_operation);
        }

        status_lines.join("\n")
    }

    /// Format a single operation for display
    fn format_operation_display(op: &OperationStatus) -> String {
        let elapsed = op.started_at.elapsed().unwrap_or_default();

        let (status_icon, status_info) = if op.is_cancelled {
            ("🛑", "Cancelled".to_string())
        } else if op.is_failed {
            ("❌", op.message.clone())
        } else if op.is_waiting {
            ("⏳", op.message.clone())
        } else if Self::should_show_progress_bar(op.current, op.total) {
            ("🔄", Self::create_progress_bar(op.current, op.total, &op.message))
        } else {
            ("🔄", op.message.clone())
        };

        let operation_desc = op.operation_type.display_name();

        // Format with conditional elapsed time and ETA
        if op.is_cancelled || op.is_failed {
            format!(
                "  {} {} | {}\n    {}",
                status_icon, op.short_id, operation_desc, status_info
            )
        } else {
            let mut time_info = format!("Elapsed: {}s", elapsed.as_secs());

            if let Some(eta) = op.eta {
                time_info.push_str(&format!(" | ETA: {}s", eta.as_secs()));
            }

            format!(
                "  {} {} | {}\n    {} | {}",
                status_icon, op.short_id, operation_desc, status_info, time_info
            )
        }
    }

    /// Check if progress bar should be shown
    fn should_show_progress_bar(current: u64, total: u64) -> bool {
        total > 0 && current <= total
    }

    /// Create progress bar display
    fn create_progress_bar(current: u64, total: u64, message: &str) -> String {
        if total == 0 {
            return message.to_string();
        }

        let percentage = (current as f64 / total as f64 * 100.0) as u8;
        let filled = (current as f64 / total as f64 * 30.0) as usize;
        let empty = 30 - filled;

        let mut bar = String::new();
        bar.push_str(&"█".repeat(filled));
        if filled < 30 && current < total {
            bar.push('▓');
            bar.push_str(&"░".repeat(empty.saturating_sub(1)));
        } else {
            bar.push_str(&"░".repeat(empty));
        }

        format!("{} {}% ({}/{}) {}", bar, percentage, current, total, message)
    }

    /// Handle cancel operation
    async fn handle_cancel(operation_id: Option<&str>) -> OperationResult {
        let async_knowledge_store = KnowledgeStore::get_async_instance().await;
        let mut store = async_knowledge_store.lock().await;

        match store.cancel_operation(operation_id).await {
            Ok(result) => OperationResult::Success(result),
            Err(e) => OperationResult::Error(format!("Failed to cancel operation: {}", e)),
        }
    }

    /// Validate and sanitize path
    fn validate_and_sanitize_path(ctx: &Context, path: &str) -> Result<String, String> {
        if path.contains('\n') {
            return Ok(path.to_string());
        }

        let ctx_path = sanitize_path_tool_arg(ctx, path);
        if !ctx_path.exists() {
            return Err(format!("Path '{}' does not exist", path));
        }

        Ok(ctx_path.to_string_lossy().to_string())
    }

    fn write_operation_result(output: &mut impl Write, result: OperationResult) -> Result<(), std::io::Error> {
        match result {
            OperationResult::Success(msg) => {
                queue!(
                    output,
                    style::SetForegroundColor(Color::Green),
                    style::Print(format!("\n{}\n\n", msg)),
                    style::SetForegroundColor(Color::Reset)
                )
            },
            OperationResult::Info(msg) => {
                if !msg.trim().is_empty() {
                    queue!(
                        output,
                        style::Print(format!("\n{}\n\n", msg)),
                        style::SetForegroundColor(Color::Reset)
                    )?;
                }
                Ok(())
            },
            OperationResult::Warning(msg) => {
                queue!(
                    output,
                    style::SetForegroundColor(Color::Yellow),
                    style::Print(format!("\n{}\n\n", msg)),
                    style::SetForegroundColor(Color::Reset)
                )
            },
            OperationResult::Error(msg) => {
                queue!(
                    output,
                    style::SetForegroundColor(Color::Red),
                    style::Print(format!("\nError: {}\n\n", msg)),
                    style::SetForegroundColor(Color::Reset)
                )
            },
        }
    }

    /// Get help text for knowledge commands
    pub fn help_text() -> String {
        color_print::cformat!(
            r#"
<magenta,em>(Beta) Knowledge Base Management</magenta,em>

Knowledge base allows you to store and search information across chat sessions.
Files and directories added to the knowledge base are indexed for semantic search,
enabling more relevant and contextual responses.

<cyan!>Available commands</cyan!>
  <em>help</em>                       <black!>Show an explanation for the knowledge command</black!>
  <em>show</em>                       <black!>Display the knowledge base contents</black!>
  <em>add PATH</em>                   <black!>Add a file or directory to knowledge base</black!>
  <em>update PATH</em>                <black!>Update a file or directory in knowledge base</black!>
  <em>rm PATH</em>                    <black!>Remove specified knowledge context by path</black!>
  <em>clear</em>                      <black!>Remove all knowledge contexts</black!>
  <em>status</em>                     <black!>Show background operation status</black!>
  <em>cancel [OPERATION_ID]</em>      <black!>Cancel a background operation (all if no OPERATION_ID)</black!>

<cyan!>Examples</cyan!>
  /knowledge add ./src                  Add a directory to knowledge base
  /knowledge add README.md              Add a file to knowledge base
  /knowledge show                       List all knowledge contexts
  /knowledge rm ./src                   Remove a context by path
  /knowledge status                     Check indexing progress
  /knowledge cancel                     All operations
  /knowledge cancel op_123456           Cancel specific operation by ID
"#
        )
    }
}
