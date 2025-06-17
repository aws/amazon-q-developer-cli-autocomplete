use eyre::Result;

use crate::cli::chat::consts::CONTEXT_FILES_MAX_SIZE;
use crate::cli::chat::context::ContextManager;
use crate::platform::Context;

// Helper function to create a test ContextManager with Context
pub async fn create_test_context_manager(context_file_size: Option<usize>) -> Result<ContextManager> {
    let context_file_size = context_file_size.unwrap_or(CONTEXT_FILES_MAX_SIZE);
    let ctx = Context::new();
    let manager = ContextManager::new(&ctx, Some(context_file_size)).await?;
    Ok(manager)
}
