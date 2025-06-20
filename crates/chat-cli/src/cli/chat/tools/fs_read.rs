use std::collections::VecDeque;
use std::fs::Metadata;
use std::io::Write;

use crossterm::queue;
use crossterm::style::{
    self,
    Color,
    Stylize,
};
use eyre::{
    Result,
    bail,
};
use serde::{
    Deserialize,
    Serialize,
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
    handle_images_from_paths,
    is_supported_image_type,
    pre_process,
};
use crate::platform::Context;

const CHECKMARK: &str = "✔";
const CROSS: &str = "✘";

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "mode")]
pub enum FsRead {
    Line(FsLine),
    Directory(FsDirectory),
    Search(FsSearch),
    Image(FsImage),
}

impl FsRead {
    pub async fn validate(&mut self, ctx: &Context) -> Result<()> {
        match self {
            FsRead::Line(fs_line) => fs_line.validate(ctx).await,
            FsRead::Directory(fs_directory) => fs_directory.validate(ctx).await,
            FsRead::Search(fs_search) => fs_search.validate(ctx).await,
            FsRead::Image(fs_image) => fs_image.validate(ctx).await,
        }
    }

    pub async fn queue_description(&self, ctx: &Context, updates: &mut impl Write) -> Result<()> {
        match self {
            FsRead::Line(fs_line) => fs_line.queue_description(ctx, updates).await,
            FsRead::Directory(fs_directory) => fs_directory.queue_description(updates),
            FsRead::Search(fs_search) => fs_search.queue_description(updates),
            FsRead::Image(fs_image) => fs_image.queue_description(updates),
        }
    }

    pub async fn invoke(&self, ctx: &Context, updates: &mut impl Write) -> Result<InvokeOutput> {
        match self {
            FsRead::Line(fs_line) => fs_line.invoke(ctx, updates).await,
            FsRead::Directory(fs_directory) => fs_directory.invoke(ctx, updates).await,
            FsRead::Search(fs_search) => fs_search.invoke(ctx, updates).await,
            FsRead::Image(fs_image) => fs_image.invoke(ctx, updates).await,
        }
    }
}

/// Read images from given paths.
#[derive(Debug, Clone, Deserialize)]
pub struct FsImage {
    pub image_paths: Vec<String>,
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
                let is_file = ctx.fs.symlink_metadata(&processed_path).await?.is_file();
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
        Ok(InvokeOutput {
            output: OutputKind::Images(valid_images),
        })
    }

    pub fn queue_description(&self, updates: &mut impl Write) -> Result<()> {
        queue!(
            updates,
            style::Print("Reading images: \n"),
            style::SetForegroundColor(Color::Green),
            style::Print(&self.image_paths.join("\n")),
            style::ResetColor,
        )?;
        Ok(())
    }
}

/// Read lines from a file.
#[derive(Debug, Clone, Deserialize)]
pub struct FsLine {
    pub path: String,
    pub start_line: Option<i32>,
    pub end_line: Option<i32>,
}

impl FsLine {
    const DEFAULT_END_LINE: i32 = -1;
    const DEFAULT_START_LINE: i32 = 1;

    pub async fn validate(&mut self, ctx: &Context) -> Result<()> {
        let path = sanitize_path_tool_arg(ctx, &self.path);
        if !path.exists() {
            bail!("'{}' does not exist", self.path);
        }
        let is_file = ctx.fs.symlink_metadata(&path).await?.is_file();
        if !is_file {
            bail!("'{}' is not a file", self.path);
        }
        Ok(())
    }

    pub async fn queue_description(&self, ctx: &Context, updates: &mut impl Write) -> Result<()> {
        let path = sanitize_path_tool_arg(ctx, &self.path);
        let file_bytes = ctx.fs.read(&path).await?;
        let file_content = String::from_utf8_lossy(&file_bytes);
        let line_count = file_content.lines().count();
        queue!(
            updates,
            style::Print("Reading file: "),
            style::SetForegroundColor(Color::Green),
            style::Print(&self.path),
            style::ResetColor,
            style::Print(", "),
        )?;

        let start = convert_negative_index(line_count, self.start_line()) + 1;
        let end = convert_negative_index(line_count, self.end_line()) + 1;
        match (start, end) {
            _ if start == 1 && end == line_count => Ok(queue!(updates, style::Print("all lines".to_string()))?),
            _ if end == line_count => Ok(queue!(
                updates,
                style::Print("from line "),
                style::SetForegroundColor(Color::Green),
                style::Print(start),
                style::ResetColor,
                style::Print(" to end of file"),
            )?),
            _ => Ok(queue!(
                updates,
                style::Print("from line "),
                style::SetForegroundColor(Color::Green),
                style::Print(start),
                style::ResetColor,
                style::Print(" to "),
                style::SetForegroundColor(Color::Green),
                style::Print(end),
                style::ResetColor,
            )?),
        }
    }

    pub async fn invoke(&self, ctx: &Context, _updates: &mut impl Write) -> Result<InvokeOutput> {
        let path = sanitize_path_tool_arg(ctx, &self.path);
        debug!(?path, "Reading");
        let file_bytes = ctx.fs.read(&path).await?;
        let file_content = String::from_utf8_lossy(&file_bytes);
        let line_count = file_content.lines().count();
        let (start, end) = (
            convert_negative_index(line_count, self.start_line()),
            convert_negative_index(line_count, self.end_line()),
        );

        // safety check to ensure end is always greater than start
        let end = end.max(start);

        if start >= line_count {
            bail!(
                "starting index: {} is outside of the allowed range: ({}, {})",
                self.start_line(),
                -(line_count as i64),
                line_count
            );
        }

        // The range should be inclusive on both ends.
        let file_contents = file_content
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

        Ok(InvokeOutput {
            output: OutputKind::Text(file_contents),
        })
    }

    fn start_line(&self) -> i32 {
        self.start_line.unwrap_or(Self::DEFAULT_START_LINE)
    }

    fn end_line(&self) -> i32 {
        self.end_line.unwrap_or(Self::DEFAULT_END_LINE)
    }
}

/// Search in a file.
#[derive(Debug, Clone, Deserialize)]
pub struct FsSearch {
    pub path: String,
    pub pattern: String,
    pub context_lines: Option<usize>,
}

impl FsSearch {
    const CONTEXT_LINE_PREFIX: &str = "  ";
    const DEFAULT_CONTEXT_LINES: usize = 2;
    const MATCHING_LINE_PREFIX: &str = "→ ";

    pub async fn validate(&mut self, ctx: &Context) -> Result<()> {
        let path = sanitize_path_tool_arg(ctx, &self.path);
        let relative_path = format_path(ctx.env.current_dir()?, &path);
        if !path.exists() {
            bail!("File not found: {}", relative_path);
        }
        if !ctx.fs.symlink_metadata(path).await?.is_file() {
            bail!("Path is not a file: {}", relative_path);
        }
        if self.pattern.is_empty() {
            bail!("Search pattern cannot be empty");
        }
        Ok(())
    }

    pub fn queue_description(&self, updates: &mut impl Write) -> Result<()> {
        queue!(
            updates,
            style::Print("Searching: "),
            style::SetForegroundColor(Color::Green),
            style::Print(&self.path),
            style::ResetColor,
            style::Print(" for pattern: "),
            style::SetForegroundColor(Color::Green),
            style::Print(&self.pattern.to_lowercase()),
            style::ResetColor,
            style::Print("\n"),
        )?;
        Ok(())
    }

    pub async fn invoke(&self, ctx: &Context, updates: &mut impl Write) -> Result<InvokeOutput> {
        let file_path = sanitize_path_tool_arg(ctx, &self.path);
        let pattern = &self.pattern;

        let file_bytes = ctx.fs.read(&file_path).await?;
        let file_content = String::from_utf8_lossy(&file_bytes);
        let lines: Vec<&str> = LinesWithEndings::from(&file_content).collect();

        let mut results = Vec::new();
        let mut total_matches = 0;

        // Case insensitive search
        let pattern_lower = pattern.to_lowercase();
        for (line_num, line) in lines.iter().enumerate() {
            if line.to_lowercase().contains(&pattern_lower) {
                total_matches += 1;
                let start = line_num.saturating_sub(self.context_lines());
                let end = lines.len().min(line_num + self.context_lines() + 1);
                let mut context_text = Vec::new();
                (start..end).for_each(|i| {
                    let prefix = if i == line_num {
                        Self::MATCHING_LINE_PREFIX
                    } else {
                        Self::CONTEXT_LINE_PREFIX
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
        let match_text = if total_matches == 1 {
            "1 match".to_string()
        } else {
            format!("{} matches", total_matches)
        };

        let color = if total_matches == 0 {
            Color::Yellow
        } else {
            Color::Green
        };

        let result = if total_matches == 0 {
            CROSS.yellow()
        } else {
            CHECKMARK.green()
        };

        queue!(
            updates,
            style::SetForegroundColor(Color::Yellow),
            style::ResetColor,
            style::Print(CONTINUATION_LINE),
            style::Print("\n"),
            style::Print(" "),
            style::Print(result),
            style::Print(" Found: "),
            style::SetForegroundColor(color),
            style::Print(match_text),
            style::ResetColor,
        )?;

        Ok(InvokeOutput {
            output: OutputKind::Text(serde_json::to_string(&results)?),
        })
    }

    fn context_lines(&self) -> usize {
        self.context_lines.unwrap_or(Self::DEFAULT_CONTEXT_LINES)
    }
}

/// List directory contents.
#[derive(Debug, Clone, Deserialize)]
pub struct FsDirectory {
    pub path: String,
    pub depth: Option<usize>,
}

impl FsDirectory {
    const DEFAULT_DEPTH: usize = 0;

    pub async fn validate(&mut self, ctx: &Context) -> Result<()> {
        let path = sanitize_path_tool_arg(ctx, &self.path);
        let relative_path = format_path(ctx.env.current_dir()?, &path);
        if !path.exists() {
            bail!("Directory not found: {}", relative_path);
        }
        if !ctx.fs.symlink_metadata(path).await?.is_dir() {
            bail!("Path is not a directory: {}", relative_path);
        }
        Ok(())
    }

    pub fn queue_description(&self, updates: &mut impl Write) -> Result<()> {
        queue!(
            updates,
            style::Print("Reading directory: "),
            style::SetForegroundColor(Color::Green),
            style::Print(&self.path),
            style::ResetColor,
            style::Print(" "),
        )?;
        let depth = self.depth.unwrap_or_default();
        Ok(queue!(
            updates,
            style::Print(format!("with maximum depth of {}", depth))
        )?)
    }

    pub async fn invoke(&self, ctx: &Context, _updates: &mut impl Write) -> Result<InvokeOutput> {
        let path = sanitize_path_tool_arg(ctx, &self.path);
        let max_depth = self.depth();
        debug!(?path, max_depth, "Reading directory at path with depth");
        let mut result = Vec::new();
        let mut dir_queue = VecDeque::new();
        dir_queue.push_back((path, 0));
        while let Some((path, depth)) = dir_queue.pop_front() {
            if depth > max_depth {
                break;
            }
            let mut read_dir = ctx.fs.read_dir(path).await?;

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

        Ok(InvokeOutput {
            output: OutputKind::Text(result),
        })
    }

    fn depth(&self) -> usize {
        self.depth.unwrap_or(Self::DEFAULT_DEPTH)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::chat::util::test::{
        TEST_FILE_CONTENTS,
        TEST_FILE_PATH,
        setup_test_directory,
    };

    #[test]
    fn test_negative_index_conversion() {
        assert_eq!(convert_negative_index(5, -100), 0);
        assert_eq!(convert_negative_index(5, -1), 4);
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
            serde_json::json!({ "path": "/test_file.txt", "mode": "Search", "pattern": "hello" }),
        )
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
            "pattern": "hello",
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

    #[tokio::test]
    async fn test_fs_read_non_utf8_binary_file() {
        let ctx = Context::new();
        let mut stdout = std::io::stdout();

        let binary_data = vec![0xff, 0xfe, 0xfd, 0xfc, 0xfb, 0xfa, 0xf9, 0xf8];
        let binary_file_path = "/binary_test.dat";
        ctx.fs.write(binary_file_path, &binary_data).await.unwrap();

        let v = serde_json::json!({
            "path": binary_file_path,
            "mode": "Line"
        });
        let output = serde_json::from_value::<FsRead>(v)
            .unwrap()
            .invoke(&ctx, &mut stdout)
            .await
            .unwrap();

        if let OutputKind::Text(text) = output.output {
            assert!(text.contains('�'), "Binary data should contain replacement characters");
            assert_eq!(text.chars().count(), 8, "Should have 8 replacement characters");
            assert!(
                text.chars().all(|c| c == '�'),
                "All characters should be replacement characters"
            );
        } else {
            panic!("expected text output");
        }
    }

    #[tokio::test]
    async fn test_fs_read_latin1_encoded_file() {
        let ctx = Context::new();
        let mut stdout = std::io::stdout();

        let latin1_data = vec![99, 97, 102, 233]; // "café" in Latin-1
        let latin1_file_path = "/latin1_test.txt";
        ctx.fs.write(latin1_file_path, &latin1_data).await.unwrap();

        let v = serde_json::json!({
            "path": latin1_file_path,
            "mode": "Line"
        });
        let output = serde_json::from_value::<FsRead>(v)
            .unwrap()
            .invoke(&ctx, &mut stdout)
            .await
            .unwrap();

        if let OutputKind::Text(text) = output.output {
            // Latin-1 byte 233 (é) is invalid UTF-8, so it becomes a replacement character
            assert!(text.starts_with("caf"), "Should start with 'caf'");
            assert!(
                text.contains('�'),
                "Should contain replacement character for invalid UTF-8"
            );
        } else {
            panic!("expected text output");
        }
    }

    #[tokio::test]
    async fn test_fs_search_non_utf8_file() {
        let ctx = Context::new();
        let mut stdout = std::io::stdout();

        let mut mixed_data = Vec::new();
        mixed_data.extend_from_slice(b"Hello world\n");
        mixed_data.extend_from_slice(&[0xff, 0xfe]); // Invalid UTF-8 bytes
        mixed_data.extend_from_slice(b"\nGoodbye world\n");

        let mixed_file_path = "/mixed_encoding_test.txt";
        ctx.fs.write(mixed_file_path, &mixed_data).await.unwrap();

        let v = serde_json::json!({
            "mode": "Search",
            "path": mixed_file_path,
            "pattern": "hello"
        });
        let output = serde_json::from_value::<FsRead>(v)
            .unwrap()
            .invoke(&ctx, &mut stdout)
            .await
            .unwrap();

        if let OutputKind::Text(value) = output.output {
            let matches: Vec<SearchMatch> = serde_json::from_str(&value).unwrap();
            assert_eq!(matches.len(), 1, "Should find one match for 'hello'");
            assert_eq!(matches[0].line_number, 1, "Match should be on line 1");
            assert!(
                matches[0].context.contains("Hello world"),
                "Should contain the matched line"
            );
        } else {
            panic!("expected Text output");
        }

        let v = serde_json::json!({
            "mode": "Search",
            "path": mixed_file_path,
            "pattern": "goodbye"
        });
        let output = serde_json::from_value::<FsRead>(v)
            .unwrap()
            .invoke(&ctx, &mut stdout)
            .await
            .unwrap();

        if let OutputKind::Text(value) = output.output {
            let matches: Vec<SearchMatch> = serde_json::from_str(&value).unwrap();
            assert_eq!(matches.len(), 1, "Should find one match for 'goodbye'");
            assert!(
                matches[0].context.contains("Goodbye world"),
                "Should contain the matched line"
            );
        } else {
            panic!("expected Text output");
        }
    }

    #[tokio::test]
    async fn test_fs_read_windows1252_encoded_file() {
        let ctx = Context::new();
        let mut stdout = std::io::stdout();

        let mut windows1252_data = Vec::new();
        windows1252_data.extend_from_slice(b"Text with ");
        windows1252_data.push(0x93); // Left double quotation mark in Windows-1252
        windows1252_data.extend_from_slice(b"smart quotes");
        windows1252_data.push(0x94); // Right double quotation mark in Windows-1252

        let windows1252_file_path = "/windows1252_test.txt";
        ctx.fs.write(windows1252_file_path, &windows1252_data).await.unwrap();

        let v = serde_json::json!({
            "path": windows1252_file_path,
            "mode": "Line"
        });
        let output = serde_json::from_value::<FsRead>(v)
            .unwrap()
            .invoke(&ctx, &mut stdout)
            .await
            .unwrap();

        if let OutputKind::Text(text) = output.output {
            assert!(text.contains("Text with"), "Should contain readable text");
            assert!(text.contains("smart quotes"), "Should contain readable text");
            assert!(
                text.contains('�'),
                "Should contain replacement characters for invalid UTF-8"
            );
        } else {
            panic!("expected text output");
        }
    }

    #[tokio::test]
    async fn test_fs_search_pattern_with_replacement_chars() {
        let ctx = Context::new();
        let mut stdout = std::io::stdout();

        let mut data_with_invalid_utf8 = Vec::new();
        data_with_invalid_utf8.extend_from_slice(b"Line 1: caf");
        data_with_invalid_utf8.push(0xe9); // Invalid UTF-8 byte (Latin-1 é)
        data_with_invalid_utf8.extend_from_slice(b"\nLine 2: hello world\n");

        let invalid_utf8_file_path = "/invalid_utf8_search_test.txt";
        ctx.fs
            .write(invalid_utf8_file_path, &data_with_invalid_utf8)
            .await
            .unwrap();

        let v = serde_json::json!({
            "mode": "Search",
            "path": invalid_utf8_file_path,
            "pattern": "caf"
        });
        let output = serde_json::from_value::<FsRead>(v)
            .unwrap()
            .invoke(&ctx, &mut stdout)
            .await
            .unwrap();

        if let OutputKind::Text(value) = output.output {
            let matches: Vec<SearchMatch> = serde_json::from_str(&value).unwrap();
            assert_eq!(matches.len(), 1, "Should find one match for 'caf'");
            assert_eq!(matches[0].line_number, 1, "Match should be on line 1");
            assert!(matches[0].context.contains("caf"), "Should contain 'caf'");
        } else {
            panic!("expected Text output");
        }
    }

    #[tokio::test]
    async fn test_fs_read_empty_file_with_invalid_utf8() {
        let ctx = Context::new();
        let mut stdout = std::io::stdout();

        let invalid_only_data = vec![0xff, 0xfe, 0xfd];
        let invalid_only_file_path = "/invalid_only_test.txt";
        ctx.fs.write(invalid_only_file_path, &invalid_only_data).await.unwrap();

        let v = serde_json::json!({
            "path": invalid_only_file_path,
            "mode": "Line"
        });
        let output = serde_json::from_value::<FsRead>(v)
            .unwrap()
            .invoke(&ctx, &mut stdout)
            .await
            .unwrap();

        if let OutputKind::Text(text) = output.output {
            assert_eq!(text.chars().count(), 3, "Should have 3 replacement characters");
            assert!(text.chars().all(|c| c == '�'), "Should be all replacement characters");
        } else {
            panic!("expected text output");
        }

        let v = serde_json::json!({
            "mode": "Search",
            "path": invalid_only_file_path,
            "pattern": "test"
        });
        let output = serde_json::from_value::<FsRead>(v)
            .unwrap()
            .invoke(&ctx, &mut stdout)
            .await
            .unwrap();

        if let OutputKind::Text(value) = output.output {
            let matches: Vec<SearchMatch> = serde_json::from_str(&value).unwrap();
            assert_eq!(
                matches.len(),
                0,
                "Should find no matches in file with only invalid UTF-8"
            );
        } else {
            panic!("expected Text output");
        }
    }
}
