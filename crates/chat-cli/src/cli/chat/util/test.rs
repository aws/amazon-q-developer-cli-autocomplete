use eyre::Result;

use crate::cli::chat::consts::CONTEXT_FILES_MAX_SIZE;
use crate::cli::chat::context::ContextManager;
use crate::platform::Context;

pub const TEST_FILE_CONTENTS: &str = "\
1: Hello world!
2: This is line 2
3: asdf
4: Hello world!
";

pub const TEST_FILE_PATH: &str = "/test_file.txt";
pub const TEST_HIDDEN_FILE_PATH: &str = "/aaaa2/.hidden";

// Helper function to create a test ContextManager with Context
pub async fn create_test_context_manager(context_file_size: Option<usize>) -> Result<ContextManager> {
    let context_file_size = context_file_size.unwrap_or(CONTEXT_FILES_MAX_SIZE);
    let ctx = Context::new();
    let manager = ContextManager::new(&ctx, Some(context_file_size)).await?;
    Ok(manager)
}

/// Sets up the following filesystem structure:
/// ```text
/// test_file.txt
/// /home/testuser/
/// /aaaa1/
///     /bbbb1/
///         /cccc1/
/// /aaaa2/
///     .hidden
/// ```
pub async fn setup_test_directory() -> Context {
    let ctx = Context::new();
    ctx.fs.write(TEST_FILE_PATH, TEST_FILE_CONTENTS).await.unwrap();
    ctx.fs.create_dir_all("/aaaa1/bbbb1/cccc1").await.unwrap();
    ctx.fs.create_dir_all("/aaaa2").await.unwrap();
    ctx.fs
        .write(TEST_HIDDEN_FILE_PATH, "this is a hidden file")
        .await
        .unwrap();
    ctx
}
