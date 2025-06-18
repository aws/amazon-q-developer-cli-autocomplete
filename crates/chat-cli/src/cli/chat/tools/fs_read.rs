use std::collections::VecDeque;
use std::fmt::Write as FmtWrite;
use std::fs::Metadata;
use std::io::Write;
use std::time::{
    SystemTime,
    UNIX_EPOCH,
};

use crossterm::queue;
use crossterm::style::{
    self,
    Color,
};
use eyre::{
    Result,
    bail,
};
use serde::{
    Deserialize,
    Serialize,
};
use sha2::{
    Digest,
    Sha256,
};
use syntect::util::LinesWithEndings;
use tracing::{
    debug,
    warn,
};

use super::{
    InvokeOutput,
    MAX_TOOL_RESPONSE_SIZE,
    OutputKind,
    format_path,
    sanitize_path_tool_arg,
};
use crate::cli::chat::CONTINUATION_LINE;
use crate::cli::chat::util::images::{
    RichImageBlocks,
    handle_images_from_paths,
    is_supported_image_type,
    pre_process,
};
use crate::platform::Context;

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum FsRead {
    Mode(FsReadMode),
    Operations(FsReadOperations),
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "mode")]
pub enum FsReadMode {
    Line(FsLine),
    Directory(FsDirectory),
    Search(FsSearch),
    Image(FsImage),
}

#[derive(Debug, Clone, Deserialize)]
pub struct FsReadOperations {
    pub file_reads: Vec<FsReadOperation>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "mode")]
pub enum FsReadOperation {
    Line(FsLineOperation),
    Directory(FsDirectoryOperation),
    Search(FsSearchOperation),
    Image(FsImage),
}

#[derive(Debug, Clone, Deserialize)]
pub struct FsLineOperation {
    pub path: String,
    pub start_line: Option<i32>,
    pub end_line: Option<i32>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FsDirectoryOperation {
    pub path: String,
    pub depth: Option<usize>,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FsSearchOperation {
    pub path: String,
    pub substring_match: String,
    pub context_lines: Option<usize>,
    pub summary: Option<String>,
}

/// Represents either a single path or multiple paths
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum PathOrPaths {
    Multiple(Vec<String>),
    Single(String),
}

impl PathOrPaths {
    pub fn is_batch(&self) -> bool {
        matches!(self, PathOrPaths::Multiple(_))
    }

    pub fn as_single(&self) -> Option<&str> {
        if let PathOrPaths::Single(s) = self {
            Some(s)
        } else {
            None
        }
    }

    pub fn as_multiple(&self) -> Option<&[String]> {
        if let PathOrPaths::Multiple(v) = self {
            Some(v)
        } else {
            None
        }
    }

    pub fn iter(&self) -> Box<dyn Iterator<Item = &String> + '_> {
        match self {
            PathOrPaths::Single(s) => Box::new(std::iter::once(s)),
            PathOrPaths::Multiple(v) => Box::new(v.iter()),
        }
    }
}

// Response for a batch of file read operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchReadResult {
    pub total_files: usize,
    pub successful_reads: usize,
    pub failed_reads: usize,
    pub results: Vec<FileReadResult>,
}

impl BatchReadResult {
    /// Create a new BatchReadResult from a vector of FileReadResult objects
    pub fn new(results: Vec<FileReadResult>) -> Self {
        let successful_reads = results.iter().filter(|r| r.success).count();
        Self {
            total_files: results.len(),
            successful_reads,
            failed_reads: results.len() - successful_reads,
            results,
        }
    }
}

/// Response for a single file read operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileReadResult {
    pub path: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
}

impl FileReadResult {
    /// Create a new successful FileReadResult with content hash and last modified timestamp
    pub fn success(path: String, content: String, metadata: Option<&Metadata>) -> Self {
        let content_hash: Option<String> = Some(hash_content(&content));
        let last_modified = metadata.and_then(|md| md.modified().ok().map(format_timestamp));

        Self {
            path,
            success: true,
            content: Some(content),
            error: None,
            content_hash,
            last_modified,
        }
    }

    /// Create a new error FileReadResult
    pub fn error(path: String, error: String) -> Self {
        Self {
            path,
            success: false,
            content: None,
            error: Some(error),
            content_hash: None,
            last_modified: None,
        }
    }
}

/// Helper function to read a file with specified line range
async fn read_file_with_lines(
    ctx: &Context,
    path_str: &str,
    start_line: Option<i32>,
    end_line: Option<i32>,
) -> Result<String> {
    let path = sanitize_path_tool_arg(ctx, path_str);
    debug!(?path, "Reading");
    let file = ctx.fs().read_to_string(&path).await?;
    let line_count = file.lines().count();

    let start = convert_negative_index(line_count, start_line.unwrap_or(FsLine::DEFAULT_START_LINE));
    let end = convert_negative_index(line_count, end_line.unwrap_or(FsLine::DEFAULT_END_LINE));

    // safety check to ensure end is always greater than start
    let end = end.max(start);

    if start >= line_count {
        bail!(
            "starting index: {} is outside of the allowed range: ({}, {})",
            start_line.unwrap_or(FsLine::DEFAULT_START_LINE),
            -(line_count as i64),
            line_count
        );
    }

    // The range should be inclusive on both ends.
    let file_contents = file
        .lines()
        .skip(start)
        .take(end - start + 1)
        .collect::<Vec<_>>()
        .join("\n");

    let byte_count = file_contents.len();
    if byte_count > MAX_TOOL_RESPONSE_SIZE {
        bail!(
            "This tool only supports reading {MAX_TOOL_RESPONSE_SIZE} bytes at a
time. You tried to read {byte_count} bytes. Try executing with fewer lines specified."
        );
    }

    Ok(file_contents)
}

/// Helper function to read a directory with specified depth
async fn read_single_directory(ctx: &Context, path_str: &str, depth: Option<usize>) -> Result<String> {
     let path = sanitize_path_tool_arg(ctx, path_str);
        let max_depth = depth.unwrap_or(FsDirectory::DEFAULT_DEPTH);
        debug!(?path, max_depth, "Reading directory at path with depth");
        let mut result = Vec::new();
        let mut dir_queue = VecDeque::new();
        dir_queue.push_back((path, 0));
        while let Some((path, depth)) = dir_queue.pop_front() {
            if depth > max_depth {
                break;
            }
            let mut read_dir = ctx.fs().read_dir(path).await?;

            #[cfg(windows)]
            while let Some(ent) = read_dir.next_entry().await? {
                let md = ent.metadata().await?;

                let modified_timestamp = md.modified()?.duration_since(std::time::UNIX_EPOCH)?.as_secs();
                let datetime = time::OffsetDateTime::from_unix_timestamp(modified_timestamp as i64).unwrap();
                let formatted_date = datetime
                    .format(time::macros::format_description!(
                        "[month repr:short] [day] [hour]:[minute]"
                    ))
                    .unwrap();

                result.push(format!(
                    "{} {} {} {}",
                    format_ftype(&md),
                    String::from_utf8_lossy(ent.file_name().as_encoded_bytes()),
                    formatted_date,
                    ent.path().to_string_lossy()
                ));

                if md.is_dir() && md.is_dir() {
                    dir_queue.push_back((ent.path(), depth + 1));
                }
            }

            #[cfg(unix)]
            while let Some(ent) = read_dir.next_entry().await? {
                use std::os::unix::fs::{
                    MetadataExt,
                    PermissionsExt,
                };

            let md = ent.metadata().await?;
            let formatted_mode = format_mode(md.permissions().mode()).into_iter().collect::<String>();

                let modified_timestamp = md.modified()?.duration_since(std::time::UNIX_EPOCH)?.as_secs();
                let datetime = time::OffsetDateTime::from_unix_timestamp(modified_timestamp as i64).unwrap();
                let formatted_date = datetime
                    .format(time::macros::format_description!(
                        "[month repr:short] [day] [hour]:[minute]"
                    ))
                    .unwrap();

                // Mostly copying "The Long Format" from `man ls`.
                // TODO: query user/group database to convert uid/gid to names?
                result.push(format!(
                    "{}{} {} {} {} {} {} {}",
                    format_ftype(&md),
                    formatted_mode,
                    md.nlink(),
                    md.uid(),
                    md.gid(),
                    md.size(),
                    formatted_date,
                    ent.path().to_string_lossy()
                ));
                if md.is_dir() {
                    dir_queue.push_back((ent.path(), depth + 1));
                }
            }
        }

        let file_count = result.len();
        let result = result.join("\n");
        let byte_count = result.len();
        if byte_count > MAX_TOOL_RESPONSE_SIZE {
            bail!(
                "This tool only supports reading up to {MAX_TOOL_RESPONSE_SIZE} bytes at a time. You tried to read {byte_count} bytes ({file_count} files). Try executing with fewer lines specified."
            );
        }

        Ok(result)

}

/// Helper function to search a file with specified substring
async fn search_single_file(
    ctx: &Context,
    path_str: &str,
    substring: &str,
    context_lines: Option<usize>,
    updates: &mut impl Write,
) -> Result<String> {
    let file_path = sanitize_path_tool_arg(ctx, path_str);
    let relative_path = format_path(ctx.env().current_dir()?, &file_path);
    let context_lines = context_lines.unwrap_or(FsSearch::DEFAULT_CONTEXT_LINES);

    let file_content = ctx.fs().read_to_string(&file_path).await?;
    let lines: Vec<&str> = LinesWithEndings::from(&file_content).collect();

    let mut results = Vec::new();
    let mut total_matches = 0;

    // Case insensitive search
    let substring_lower = substring.to_lowercase();
    for (line_num, line) in lines.iter().enumerate() {
        if line.to_lowercase().contains(&substring_lower) {
            total_matches += 1;
            let start = line_num.saturating_sub(context_lines);
            let end = lines.len().min(line_num + context_lines + 1);
            let mut context_text = Vec::new();
            (start..end).for_each(|i| {
                let prefix = if i == line_num {
                    FsSearch::MATCHING_LINE_PREFIX
                } else {
                    FsSearch::CONTEXT_LINE_PREFIX
                };
                let line_text = lines[i].to_string();
                context_text.push(format!("{}{}: {}", prefix, i + 1, line_text));
            });
            let match_text = context_text.join("");
            results.push(SearchMatch {
                line_number: line_num + 1,
                context: match_text,
            });
        }
    }

    // Format the search results summary with consistent styling
    super::queue_function_result(
        &format!(
            "Found {} matches for substring '{}' in {}",
            total_matches, substring, relative_path
        ),
        updates,
        false,
        false,
    )?;

    Ok(serde_json::to_string(&results)?)
}

impl FsRead {
    pub async fn validate(&mut self, ctx: &Context) -> Result<()> {
        match self {
            FsRead::Mode(mode) => match mode {
                FsReadMode::Line(fs_line) => fs_line.validate(ctx).await,
                FsReadMode::Directory(fs_directory) => fs_directory.validate(ctx).await,
                FsReadMode::Search(fs_search) => fs_search.validate(ctx).await,
                FsReadMode::Image(fs_image) => fs_image.validate(ctx).await,
            },
            // Batch validation – iterate through each op
            FsRead::Operations(ops) => {
                if ops.file_reads.is_empty() {
                    bail!("At least one operation must be specified");
                }
                for op in &mut ops.file_reads {
                    match op {
                        FsReadOperation::Line(l) => validate_line(ctx, &l.path).await?,
                        FsReadOperation::Directory(d) => validate_dir(ctx, &d.path).await?,
                        FsReadOperation::Search(s) => validate_search(ctx, &s.path, &s.substring_match).await?,
                        FsReadOperation::Image(img) => img.validate(ctx).await?,
                    }
                }
                Ok(())
            },
        }
    }

    pub async fn queue_description(&self, ctx: &Context, updates: &mut impl Write) -> Result<()> {
        match self {
            FsRead::Mode(mode) => match mode {
                FsReadMode::Line(fs_line) => fs_line.queue_description(ctx, updates).await,
                FsReadMode::Directory(fs_directory) => fs_directory.queue_description(updates),
                FsReadMode::Search(fs_search) => fs_search.queue_description(updates),
                FsReadMode::Image(fs_image) => fs_image.queue_description(updates),
            },
            FsRead::Operations(ops) => {
                super::queue_summary(ops.summary.as_deref(), updates, Some(2))?;

                for (idx, op) in ops.file_reads.iter().enumerate() {
                    if idx > 0 {
                        writeln!(updates)?;
                    }
                    if ops.file_reads.len() > 1 {
                        queue!(updates, style::Print(format!(" ↱ Operation {}:\n", idx + 1)))?;
                    }
                    match op {
                        FsReadOperation::Line(l) => queue_desc_line(ctx, l, updates).await?,
                        FsReadOperation::Directory(d) => queue_desc_dir(d, updates)?,
                        FsReadOperation::Search(s) => queue_desc_search(s, updates)?,
                        FsReadOperation::Image(img) => img.queue_description(updates)?,
                    }
                }
                Ok(())
            },
        }
    }

    pub async fn invoke(&self, ctx: &Context, updates: &mut impl Write) -> Result<InvokeOutput> {
        match self {
            FsRead::Mode(mode) => match mode {
                FsReadMode::Line(fs_line) => fs_line.invoke(ctx, updates).await,
                FsReadMode::Directory(fs_directory) => fs_directory.invoke(ctx, updates).await,
                FsReadMode::Search(fs_search) => fs_search.invoke(ctx, updates).await,
                FsReadMode::Image(fs_image) => fs_image.invoke(ctx, updates).await,
            },
            FsRead::Operations(ops) => {
                debug!("Executing {} operations", ops.file_reads.len());
                let mut results = Vec::with_capacity(ops.file_reads.len());
                let mut image_blocks: RichImageBlocks = Vec::new();
                let mut image_count: usize = 0;
                for op in &ops.file_reads {
                    match op {
                        FsReadOperation::Line(l) => {
                            let out = perform_line(ctx, l, updates).await?;
                            results.push(out);
                        },
                        FsReadOperation::Directory(d) => {
                            let out = perform_dir(ctx, d, updates).await?;
                            results.push(out);
                        },
                        FsReadOperation::Search(s) => {
                            let out = perform_search(ctx, s, updates).await?;
                            results.push(out);
                        },
                        FsReadOperation::Image(img) => {
                            let result = img.invoke(ctx, updates).await?;
                            if let OutputKind::Images(mut imgs) = result.output {
                                image_count += imgs.len().max(1);
                                image_blocks.append(&mut imgs);
                            }
                        },
                    }
                }

                let batch_result = BatchReadResult::new(results);
                queue!(
                    updates,
                    style::Print("\n"),
                    style::Print(CONTINUATION_LINE),
                    style::Print("\n")
                )?;

                if batch_result.total_files + image_count > 1 {
                    super::queue_function_result(
                        &format!(
                            "Summary: {} files processed, {} successful, {} failed",
                            batch_result.total_files + image_count,
                            batch_result.successful_reads + image_count,
                            batch_result.failed_reads
                        ),
                        updates,
                        false,
                        true,
                    )?;
                }

                if !image_blocks.is_empty() {
                    return Ok(InvokeOutput {
                        output: OutputKind::ImagesAndText {
                            images: image_blocks,
                            text: serde_json::to_string(&batch_result)?,
                        },
                    });
                }

                // If there's only one operation and it's not an image, return its content directly
                if batch_result.total_files == 1 && batch_result.successful_reads == 1 {
                    if let Some(content) = &batch_result.results[0].content {
                        return Ok(InvokeOutput {
                            output: OutputKind::Text(content.clone()),
                        });
                    }
                }

                // For multiple operations or failed operations, return the BatchReadResult
                Ok(InvokeOutput {
                    output: OutputKind::Text(serde_json::to_string(&batch_result)?),
                })
            },
        }
    }
}

/// Read images from given paths.
#[derive(Debug, Clone, Deserialize)]
pub struct FsImage {
    pub image_paths: Vec<String>,
    pub summary: Option<String>,
}

impl FsImage {
    pub async fn validate(&mut self, ctx: &Context) -> Result<()> {
        for path in &self.image_paths {
            let path = sanitize_path_tool_arg(ctx, path);
            if let Some(path) = path.to_str() {
                let processed_path = pre_process(ctx, path);
                if !is_supported_image_type(&processed_path) {
                    bail!("'{}' is not a supported image type", &processed_path);
                }
                let is_file = ctx.fs().symlink_metadata(&processed_path).await?.is_file();
                if !is_file {
                    bail!("'{}' is not a file", &processed_path);
                }
            } else {
                bail!("Unable to parse path");
            }
        }
        Ok(())
    }

    pub async fn invoke(&self, ctx: &Context, updates: &mut impl Write) -> Result<InvokeOutput> {
        let pre_processed_paths: Vec<String> = self.image_paths.iter().map(|path| pre_process(ctx, path)).collect();
        let valid_images = handle_images_from_paths(updates, &pre_processed_paths);
        super::queue_function_result("Successfully read image", updates, false, false)?;
        Ok(InvokeOutput {
            output: OutputKind::Images(valid_images),
        })
    }

    pub fn queue_description(&self, updates: &mut impl Write) -> Result<()> {
        queue!(
            updates,
            style::Print("   Reading images: \n"),
            style::SetForegroundColor(Color::Green),
            style::Print(&self.image_paths.join("\n")),
            style::ResetColor,
        )?;

        // Add the summary if available
        super::queue_summary(self.summary.as_deref(), updates, None)?;

        Ok(())
    }
}

/// Read lines from a file or multiple files.
#[derive(Debug, Clone, Deserialize)]
pub struct FsLine {
    pub path: PathOrPaths,
    pub start_line: Option<i32>,
    pub end_line: Option<i32>,
    pub summary: Option<String>,
}

impl FsLine {
    const DEFAULT_END_LINE: i32 = -1;
    const DEFAULT_START_LINE: i32 = 1;

    pub async fn validate(&mut self, ctx: &Context) -> Result<()> {
        for path_str in self.path.iter() {
            validate_line(ctx, path_str).await?;
        }
        Ok(())
    }

    pub async fn queue_description(&self, ctx: &Context, updates: &mut impl Write) -> Result<()> {
        if self.path.is_batch() {
            let paths = self.path.as_multiple().unwrap();
            queue!(
                updates,
                style::Print("Reading multiple files: "),
                style::SetForegroundColor(Color::Green),
                style::Print(format!("{} files", paths.len())),
                style::ResetColor,
            )?;

            // Add the summary if available
            super::queue_summary(self.summary.as_deref(), updates, None)?;

            return Ok(());
        }

        queue_desc_line(
            ctx,
            &FsLineOperation {
                path: self.path.as_single().unwrap().to_string(),
                start_line: Some(self.start_line()),
                end_line: Some(self.end_line()),
                summary: self.summary.clone(),
            },
            updates,
        )
        .await?;
        Ok(())
    }

    pub async fn invoke(&self, ctx: &Context, _updates: &mut impl Write) -> Result<InvokeOutput> {
        // Handle batch operation
        if self.path.is_batch() {
            let paths = self.path.as_multiple().unwrap();
            let mut results = Vec::with_capacity(paths.len());

            for path_str in paths {
                let path = sanitize_path_tool_arg(ctx, path_str);
                let result = read_file_with_lines(ctx, path_str, self.start_line, self.end_line).await;
                match result {
                    Ok(content) => {
                        // Get file metadata for hash and last modified timestamp
                        let metadata = ctx.fs().symlink_metadata(&path).await.ok();
                        results.push(FileReadResult::success(path_str.clone(), content, metadata.as_ref()));
                    },
                    Err(err) => {
                        results.push(FileReadResult::error(path_str.clone(), err.to_string()));
                    },
                }
            }

            // Create a BatchReadResult from the results
            let batch_result = BatchReadResult::new(results);
            return Ok(InvokeOutput {
                output: OutputKind::Text(serde_json::to_string(&batch_result)?),
            });
        }

        // Handle single file operation
        let path_str = self.path.as_single().unwrap();
        match read_file_with_lines(ctx, path_str, self.start_line, self.end_line).await {
            Ok(file_contents) => {
                // Get file metadata for hash and last modified timestamp
                let path = sanitize_path_tool_arg(ctx, path_str);
                let _metadata = ctx.fs().symlink_metadata(&path).await.ok();

                // For single file operations, return content directly for backward compatibility
                Ok(InvokeOutput {
                    output: OutputKind::Text(file_contents),
                })
            },
            Err(err) => Err(err),
        }
    }

    fn start_line(&self) -> i32 {
        self.start_line.unwrap_or(Self::DEFAULT_START_LINE)
    }

    fn end_line(&self) -> i32 {
        self.end_line.unwrap_or(Self::DEFAULT_END_LINE)
    }
}

/// Search in a file or multiple files.
#[derive(Debug, Clone, Deserialize)]
pub struct FsSearch {
    pub path: PathOrPaths,
    pub substring_match: String,
    pub context_lines: Option<usize>,
    pub summary: Option<String>,
}

impl FsSearch {
    const CONTEXT_LINE_PREFIX: &str = "  ";
    const DEFAULT_CONTEXT_LINES: usize = 2;
    const MATCHING_LINE_PREFIX: &str = "→ ";

    pub async fn validate(&mut self, ctx: &Context) -> Result<()> {
        for path_str in self.path.iter() {
            validate_search(ctx, path_str, &self.substring_match).await?;
        }
        Ok(())
    }

    pub fn queue_description(&self, updates: &mut impl Write) -> Result<()> {
        if self.path.is_batch() {
            let paths = self.path.as_multiple().unwrap();
            queue!(
                updates,
                style::Print("Searching multiple files: "),
                style::SetForegroundColor(Color::Green),
                style::Print(format!("{} files", paths.len())),
                style::ResetColor,
                style::Print(" for substring: "),
                style::SetForegroundColor(Color::Green),
                style::Print(&self.substring_match.to_lowercase()),
                style::ResetColor,
                style::Print("\n"),
            )?;

            // Add the summary if available
            super::queue_summary(self.summary.as_deref(), updates, None)?;

            return Ok(());
        }

        let _ = queue_desc_search(
            &FsSearchOperation {
                path: self.path.as_single().unwrap().to_string(),
                substring_match: self.substring_match.to_lowercase(),
                context_lines: self.context_lines,
                summary: self.summary.clone(),
            },
            updates,
        );
        Ok(())
    }

    pub async fn invoke(&self, ctx: &Context, updates: &mut impl Write) -> Result<InvokeOutput> {
        // Handle batch operation
        if self.path.is_batch() {
            let paths = self.path.as_multiple().unwrap();
            let mut results = Vec::with_capacity(paths.len());

            for path_str in paths {
                let path = sanitize_path_tool_arg(ctx, path_str);
                let result = search_single_file(
                    ctx,
                    path_str,
                    &self.substring_match,
                    Some(self.context_lines()),
                    updates,
                )
                .await;
                match result {
                    Ok(content) => {
                        // Get file metadata for hash and last modified timestamp
                        let metadata = ctx.fs().symlink_metadata(&path).await.ok();
                        results.push(FileReadResult::success(path_str.clone(), content, metadata.as_ref()));
                    },
                    Err(err) => {
                        results.push(FileReadResult::error(path_str.clone(), err.to_string()));
                    },
                }
            }

            // Create a BatchReadResult from the results
            let batch_result = BatchReadResult::new(results);
            return Ok(InvokeOutput {
                output: OutputKind::Text(serde_json::to_string(&batch_result)?),
            });
        }

        // Handle single file operation
        let path_str = self.path.as_single().unwrap();
        match search_single_file(
            ctx,
            path_str,
            &self.substring_match,
            Some(self.context_lines()),
            updates,
        )
        .await
        {
            Ok(search_results) => {
                // For single file operations, return content directly for backward compatibility
                Ok(InvokeOutput {
                    output: OutputKind::Text(search_results),
                })
            },
            Err(err) => Err(err),
        }
    }

    fn context_lines(&self) -> usize {
        self.context_lines.unwrap_or(Self::DEFAULT_CONTEXT_LINES)
    }
}

/// List directory contents.
#[derive(Debug, Clone, Deserialize)]
pub struct FsDirectory {
    pub path: PathOrPaths,
    pub depth: Option<usize>,
    pub summary: Option<String>,
}

impl FsDirectory {
    const DEFAULT_DEPTH: usize = 0;

    pub async fn validate(&mut self, ctx: &Context) -> Result<()> {
        for path_str in self.path.iter() {
            validate_dir(ctx, path_str).await?;
        }
        Ok(())
    }

    pub fn queue_description(&self, updates: &mut impl Write) -> Result<()> {
        if self.path.is_batch() {
            let paths = self.path.as_multiple().unwrap();
            queue!(
                updates,
                style::Print("Reading multiple directories: "),
                style::SetForegroundColor(Color::Green),
                style::Print(format!("{} directories", paths.len())),
                style::ResetColor,
                style::Print(" "),
            )?;
            let depth = self.depth.unwrap_or_default();
            queue!(updates, style::Print(format!("with maximum depth of {}", depth)))?;

            // Add the summary if available
            super::queue_summary(self.summary.as_deref(), updates, None)?;

            return Ok(());
        }

        let _ = queue_desc_dir(
            &FsDirectoryOperation {
                path: self.path.as_single().unwrap().to_string(),
                depth: self.depth,
                summary: self.summary.clone(),
            },
            updates,
        );
        Ok(())
    }

    pub async fn invoke(&self, ctx: &Context, _updates: &mut impl Write) -> Result<InvokeOutput> {
        // Handle batch operation
        if self.path.is_batch() {
            let paths = self.path.as_multiple().unwrap();
            let mut results = Vec::with_capacity(paths.len());

            for path_str in paths {
                let path = sanitize_path_tool_arg(ctx, path_str);
                let result = read_single_directory(ctx, path_str, Some(self.depth())).await;
                match result {
                    Ok(content) => {
                        // Get directory metadata for last modified timestamp
                        let metadata = ctx.fs().symlink_metadata(&path).await.ok();
                        results.push(FileReadResult::success(path_str.clone(), content, metadata.as_ref()));
                    },
                    Err(err) => {
                        results.push(FileReadResult::error(path_str.clone(), err.to_string()));
                    },
                }
            }

            // Create a BatchReadResult from the results
            let batch_result = BatchReadResult::new(results);
            return Ok(InvokeOutput {
                output: OutputKind::Text(serde_json::to_string(&batch_result)?),
            });
        }

        // Handle single directory operation
        let path_str = self.path.as_single().unwrap();
        match read_single_directory(ctx, path_str, Some(self.depth())).await {
            Ok(directory_contents) => {
                // For single directory operations, return content directly for backward compatibility
                Ok(InvokeOutput {
                    output: OutputKind::Text(directory_contents),
                })
            },
            Err(err) => Err(err),
        }
    }

    fn depth(&self) -> usize {
        self.depth.unwrap_or(Self::DEFAULT_DEPTH)
    }
}

/// Generate a SHA-256 hash of the content
fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();

    // Convert to hex string
    let mut s = String::with_capacity(result.len() * 2);
    for b in result {
        let _ = FmtWrite::write_fmt(&mut s, format_args!("{:02x}", b));
    }
    s
}

/// Format a SystemTime as an ISO 8601 UTC timestamp
fn format_timestamp(time: SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs();
    let nanos = duration.subsec_nanos();

    // Use time crate to format the timestamp
    let datetime = time::OffsetDateTime::from_unix_timestamp(secs as i64)
        .unwrap()
        .replace_nanosecond(nanos)
        .unwrap();

    datetime.format(&time::format_description::well_known::Rfc3339).unwrap()
}

/// Converts negative 1-based indices to positive 0-based indices.
fn convert_negative_index(line_count: usize, i: i32) -> usize {
    if i <= 0 {
        (line_count as i32 + i).max(0) as usize
    } else {
        i as usize - 1
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SearchMatch {
    line_number: usize,
    context: String,
}

fn format_ftype(md: &Metadata) -> char {
    if md.is_symlink() {
        'l'
    } else if md.is_file() {
        '-'
    } else if md.is_dir() {
        'd'
    } else {
        warn!("unknown file metadata: {:?}", md);
        '-'
    }
}

/// Formats a permissions mode into the form used by `ls`, e.g. `0o644` to `rw-r--r--`
#[cfg(unix)]
fn format_mode(mode: u32) -> [char; 9] {
    let mut mode = mode & 0o777;
    let mut res = ['-'; 9];
    fn octal_to_chars(val: u32) -> [char; 3] {
        match val {
            1 => ['-', '-', 'x'],
            2 => ['-', 'w', '-'],
            3 => ['-', 'w', 'x'],
            4 => ['r', '-', '-'],
            5 => ['r', '-', 'x'],
            6 => ['r', 'w', '-'],
            7 => ['r', 'w', 'x'],
            _ => ['-', '-', '-'],
        }
    }
    for c in res.rchunks_exact_mut(3) {
        c.copy_from_slice(&octal_to_chars(mode & 0o7));
        mode /= 0o10;
    }
    res
}

///  Validation helpers for read operations
async fn validate_line(ctx: &Context, path_str: &str) -> Result<()> {
    let path = sanitize_path_tool_arg(ctx, path_str);
    if !path.exists() {
        bail!("'{}' does not exist", path_str);
    }
    if !ctx.fs().symlink_metadata(&path).await?.is_file() {
        bail!("'{}' is not a file", path_str);
    }
    Ok(())
}

async fn validate_dir(ctx: &Context, path_str: &str) -> Result<()> {
    let path = sanitize_path_tool_arg(ctx, path_str);
    let rel = format_path(ctx.env().current_dir()?, &path);
    if !path.exists() {
        bail!("Directory not found: {}", rel);
    }
    if !ctx.fs().symlink_metadata(&path).await?.is_dir() {
        bail!("Path is not a directory: {}", rel);
    }
    Ok(())
}

async fn validate_search(ctx: &Context, path_str: &str, substring_match: &str) -> Result<()> {
    if substring_match.is_empty() {
        bail!("Search substring cannot be empty");
    }
    let path = sanitize_path_tool_arg(ctx, path_str);
    let rel = format_path(ctx.env().current_dir()?, &path);
    if !path.exists() {
        bail!("File not found: {}", rel);
    }
    if !ctx.fs().symlink_metadata(&path).await?.is_file() {
        bail!("Path is not a file: {}", rel);
    }
    Ok(())
}

///  Queue description helpers
async fn queue_desc_line(ctx: &Context, op: &FsLineOperation, updates: &mut impl Write) -> Result<()> {
    queue!(
        updates,
        style::Print("   Reading file: "),
        style::SetForegroundColor(Color::Green),
        style::Print(&op.path),
        style::ResetColor,
        style::Print(", ")
    )?;
    // Add operation-specific summary if available
    super::queue_summary(op.summary.as_deref(), updates, None)?;
    let path = sanitize_path_tool_arg(ctx, &op.path);
    let total = ctx.fs().read_to_string(&path).await?.lines().count();
    let start = convert_negative_index(total, op.start_line.unwrap_or(FsLine::DEFAULT_START_LINE)) + 1;
    let end = convert_negative_index(total, op.end_line.unwrap_or(FsLine::DEFAULT_END_LINE)) + 1;
    match (start, end) {
        (1, x) if x == total => queue!(updates, style::Print("all lines"))?,
        (start, x) if x == total => queue!(
            updates,
            style::Print("from line "),
            style::SetForegroundColor(Color::Green),
            style::Print(start),
            style::ResetColor,
            style::Print(" to end of file"),
        )?,
        _ => queue!(
            updates,
            style::Print("from line "),
            style::SetForegroundColor(Color::Green),
            style::Print(start),
            style::ResetColor,
            style::Print(" to "),
            style::SetForegroundColor(Color::Green),
            style::Print(end),
            style::ResetColor,
        )?,
    }
    Ok(())
}

fn queue_desc_dir(op: &FsDirectoryOperation, updates: &mut impl Write) -> Result<()> {
    queue!(
        updates,
        style::Print("   Reading directory: "),
        style::SetForegroundColor(Color::Green),
        style::Print(&op.path),
        style::ResetColor,
        style::Print(" with maximum depth of "),
        style::SetForegroundColor(Color::Green),
        style::Print(op.depth.unwrap_or(0)),
        style::ResetColor
    )?;
    super::queue_summary(op.summary.as_deref(), updates, None)?;
    Ok(())
}

fn queue_desc_search(op: &FsSearchOperation, updates: &mut impl Write) -> Result<()> {
    queue!(
        updates,
        style::Print("   Searching: "),
        style::SetForegroundColor(Color::Green),
        style::Print(&op.path),
        style::ResetColor,
        style::Print(" for substring: "),
        style::SetForegroundColor(Color::Green),
        style::Print(&op.substring_match.to_lowercase()),
        style::ResetColor
    )?;
    super::queue_summary(op.summary.as_deref(), updates, None)?;
    Ok(())
}

/// Execution helpers
async fn perform_line(ctx: &Context, op: &FsLineOperation, updates: &mut impl Write) -> Result<FileReadResult> {
    match read_file_with_lines(ctx, &op.path, op.start_line, op.end_line).await {
        Ok(content) => {
            let metadata = ctx
                .fs()
                .symlink_metadata(sanitize_path_tool_arg(ctx, &op.path))
                .await
                .ok();
            super::queue_function_result(
                &format!("Successfully read {} bytes from {}", content.len(), &op.path),
                updates,
                false,
                false,
            )?;
            Ok(FileReadResult::success(op.path.clone(), content, metadata.as_ref()))
        },
        Err(e) => {
            super::queue_function_result(&format!("Error reading {}: {}", &op.path, e), updates, true, false)?;
            Ok(FileReadResult::error(op.path.clone(), e.to_string()))
        },
    }
}

async fn perform_dir(ctx: &Context, op: &FsDirectoryOperation, updates: &mut impl Write) -> Result<FileReadResult> {
    match read_single_directory(ctx, &op.path, op.depth).await {
        Ok(content) => {
            let metadata = ctx
                .fs()
                .symlink_metadata(sanitize_path_tool_arg(ctx, &op.path))
                .await
                .ok();
            // Format the success message with consistent styling
            super::queue_function_result(
                &format!(
                    "Successfully read directory {} ({} entries)",
                    &op.path,
                    content.lines().count()
                ),
                updates,
                false,
                false,
            )?;
            Ok(FileReadResult::success(op.path.clone(), content, metadata.as_ref()))
        },
        Err(e) => {
            // Format the error message with consistent styling
            super::queue_function_result(
                &format!("Error reading directory {}: {}", &op.path, e),
                updates,
                true,
                false,
            )?;
            Ok(FileReadResult::error(op.path.clone(), e.to_string()))
        },
    }
}

async fn perform_search(ctx: &Context, op: &FsSearchOperation, updates: &mut impl Write) -> Result<FileReadResult> {
    match search_single_file(ctx, &op.path, &op.substring_match, op.context_lines, updates).await {
        Ok(content) => {
            let metadata = ctx
                .fs()
                .symlink_metadata(sanitize_path_tool_arg(ctx, &op.path))
                .await
                .ok();
            Ok(FileReadResult::success(op.path.clone(), content, metadata.as_ref()))
        },
        Err(e) => {
            super::queue_function_result(&format!("Error searching {}: {}", &op.path, e), updates, true, false)?;
            Ok(FileReadResult::error(op.path.clone(), e.to_string()))
        },
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    const TEST_FILE_CONTENTS: &str = "\
1: Hello world!
2: This is line 2
3: asdf
4: Hello world!
";

    const TEST_FILE_PATH: &str = "/test_file.txt";
    const TEST_HIDDEN_FILE_PATH: &str = "/aaaa2/.hidden";

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
    async fn setup_test_directory() -> Arc<Context> {
        let ctx = Context::builder().with_test_home().await.unwrap().build_fake();
        let fs = ctx.fs();
        fs.write(TEST_FILE_PATH, TEST_FILE_CONTENTS).await.unwrap();
        fs.create_dir_all("/aaaa1/bbbb1/cccc1").await.unwrap();
        fs.create_dir_all("/aaaa2").await.unwrap();
        fs.write(TEST_HIDDEN_FILE_PATH, "this is a hidden file").await.unwrap();
        ctx
    }

    #[test]
    fn test_negative_index_conversion() {
        assert_eq!(convert_negative_index(5, -100), 0);
        assert_eq!(convert_negative_index(5, -1), 4);
    }

    #[test]
    fn test_path_or_paths() {
        // Test single path
        let single = PathOrPaths::Single("test.txt".to_string());
        assert!(!single.is_batch());
        assert_eq!(single.as_single(), Some("test.txt"));
        assert_eq!(single.as_multiple(), None);
        let paths: Vec<String> = single.iter().cloned().collect();
        assert_eq!(paths, vec!["test.txt".to_string()]);

        // Test multiple paths
        let multiple = PathOrPaths::Multiple(vec!["test1.txt".to_string(), "test2.txt".to_string()]);
        assert!(multiple.is_batch());
        assert_eq!(multiple.as_single(), None);
        assert_eq!(
            multiple.as_multiple(),
            Some(&["test1.txt".to_string(), "test2.txt".to_string()][..])
        );
        let paths: Vec<String> = multiple.iter().cloned().collect();
        assert_eq!(paths, vec!["test1.txt".to_string(), "test2.txt".to_string()]);
    }

    #[test]
    fn test_fs_read_deser() {
        serde_json::from_value::<FsRead>(serde_json::json!({ "path": "/test_file.txt", "mode": "Line" })).unwrap();
        serde_json::from_value::<FsRead>(
            serde_json::json!({ "path": "/test_file.txt", "mode": "Line", "end_line": 5 }),
        )
        .unwrap();
        serde_json::from_value::<FsRead>(
            serde_json::json!({ "path": "/test_file.txt", "mode": "Line", "start_line": -1 }),
        )
        .unwrap();
        serde_json::from_value::<FsRead>(
            serde_json::json!({ "path": "/test_file.txt", "mode": "Line", "start_line": None::<usize> }),
        )
        .unwrap();
        serde_json::from_value::<FsRead>(serde_json::json!({ "path": "/", "mode": "Directory" })).unwrap();
        serde_json::from_value::<FsRead>(
            serde_json::json!({ "path": "/test_file.txt", "mode": "Directory", "depth": 2 }),
        )
        .unwrap();
        serde_json::from_value::<FsRead>(
            serde_json::json!({ "path": "/test_file.txt", "mode": "Search", "substring_match": "hello" }),
        )
        .unwrap();
        serde_json::from_value::<FsRead>(serde_json::json!({
            "summary": "batch demo",
            "file_reads": [
                { "mode": "Line", "path": "/a.txt", "start_line": 1, "end_line": 10 },
                { "mode": "Directory", "path": "/",   "depth": 1 },
                { "mode": "Search", "path": "/b.txt", "substring_match": "foo" }
            ]
        }))
        .unwrap();
        serde_json::from_value::<FsRead>(serde_json::json!({
            "mode": "Line",
            "path": ["/x.txt", "/y.txt"]
        }))
        .unwrap();
    }

    #[tokio::test]
    async fn test_fs_read_line_invoke() {
        let ctx = setup_test_directory().await;
        let lines = TEST_FILE_CONTENTS.lines().collect::<Vec<_>>();
        let mut stdout = std::io::stdout();

        macro_rules! assert_lines {
            ($start_line:expr, $end_line:expr, $expected:expr) => {
                let v = serde_json::json!({
                    "path": TEST_FILE_PATH,
                    "mode": "Line",
                    "start_line": $start_line,
                    "end_line": $end_line,
                });
                let output = serde_json::from_value::<FsRead>(v)
                    .unwrap()
                    .invoke(&ctx, &mut stdout)
                    .await
                    .unwrap();

                if let OutputKind::Text(text) = output.output {
                    assert_eq!(text, $expected.join("\n"), "actual(left) does not equal
                                expected(right) for (start_line, end_line): ({:?}, {:?})", $start_line, $end_line);
                } else {
                    panic!("expected text output");
                }
            }
        }
        assert_lines!(None::<i32>, None::<i32>, lines[..]);
        assert_lines!(1, 2, lines[..=1]);
        assert_lines!(1, -1, lines[..]);
        assert_lines!(2, 1, lines[1..=1]);
        assert_lines!(-2, -1, lines[2..]);
        assert_lines!(-2, None::<i32>, lines[2..]);
        assert_lines!(2, None::<i32>, lines[1..]);
    }

    #[tokio::test]
    async fn test_fs_read_line_past_eof() {
        let ctx = setup_test_directory().await;
        let mut stdout = std::io::stdout();
        let v = serde_json::json!({
            "path": TEST_FILE_PATH,
            "mode": "Line",
            "start_line": 100,
            "end_line": None::<i32>,
        });
        assert!(
            serde_json::from_value::<FsRead>(v)
                .unwrap()
                .invoke(&ctx, &mut stdout)
                .await
                .is_err()
        );
    }

    #[test]
    #[cfg(unix)]
    fn test_format_mode() {
        macro_rules! assert_mode {
            ($actual:expr, $expected:expr) => {
                assert_eq!(format_mode($actual).iter().collect::<String>(), $expected);
            };
        }
        assert_mode!(0o000, "---------");
        assert_mode!(0o700, "rwx------");
        assert_mode!(0o744, "rwxr--r--");
        assert_mode!(0o641, "rw-r----x");
    }

    #[tokio::test]
    async fn test_fs_read_directory_invoke() {
        let ctx = setup_test_directory().await;
        let mut stdout = std::io::stdout();

        // Testing without depth
        let v = serde_json::json!({
            "mode": "Directory",
            "path": "/",
        });
        let output = serde_json::from_value::<FsRead>(v)
            .unwrap()
            .invoke(&ctx, &mut stdout)
            .await
            .unwrap();

        if let OutputKind::Text(text) = output.output {
            assert_eq!(text.lines().collect::<Vec<_>>().len(), 4);
        } else {
            panic!("expected text output");
        }

        // Testing with depth level 1
        let v = serde_json::json!({
            "mode": "Directory",
            "path": "/",
            "depth": 1,
        });
        let output = serde_json::from_value::<FsRead>(v)
            .unwrap()
            .invoke(&ctx, &mut stdout)
            .await
            .unwrap();

        if let OutputKind::Text(text) = output.output {
            let lines = text.lines().collect::<Vec<_>>();
            assert_eq!(lines.len(), 7);
            assert!(
                !lines.iter().any(|l| l.contains("cccc1")),
                "directory at depth level 2 should not be included in output"
            );
        } else {
            panic!("expected text output");
        }
    }

    #[tokio::test]
    async fn test_fs_read_search_invoke() {
        let ctx = setup_test_directory().await;
        let mut stdout = std::io::stdout();

        macro_rules! invoke_search {
            ($value:tt) => {{
                let v = serde_json::json!($value);
                let output = serde_json::from_value::<FsRead>(v)
                    .unwrap()
                    .invoke(&ctx, &mut stdout)
                    .await
                    .unwrap();

                if let OutputKind::Text(value) = output.output {
                    serde_json::from_str::<Vec<SearchMatch>>(&value).unwrap()
                } else {
                    panic!("expected Text output")
                }
            }};
        }

        let matches = invoke_search!({
            "mode": "Search",
            "path": TEST_FILE_PATH,
            "substring_match": "hello",
        });
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].line_number, 1);
        assert_eq!(
            matches[0].context,
            format!(
                "{}1: 1: Hello world!\n{}2: 2: This is line 2\n{}3: 3: asdf\n",
                FsSearch::MATCHING_LINE_PREFIX,
                FsSearch::CONTEXT_LINE_PREFIX,
                FsSearch::CONTEXT_LINE_PREFIX
            )
        );
    }
}
