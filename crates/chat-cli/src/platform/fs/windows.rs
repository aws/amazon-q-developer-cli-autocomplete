use std::fs::metadata;
use std::io;
use std::path::{
    Component,
    Path,
    PathBuf,
};

use tracing::warn;

/// Performs `a.join(b)`, except:
/// - if `b` is an absolute path, then the resulting path will equal `/a/b`
/// - if the prefix of `b` contains some `n` copies of a, then the resulting path will equal `/a/b`
pub(super) fn append(a: impl AsRef<Path>, b: impl AsRef<Path>) -> PathBuf {
    let a = a.as_ref();
    let b = b.as_ref();

    // If b is an absolute path with a Windows drive letter, handle it specially
    if b.is_absolute() {
        // First, try to strip any common prefix
        if let Ok(stripped) = b.strip_prefix(a) {
            return a.join(stripped);
        }
        
        // If that fails, we need to handle Windows drive letter paths
        // Get the non-prefix part of the path (everything after C:\)
        let mut components = b.components();
        
        // Skip the prefix (drive letter) and root (\ after C:)
        // and create a new path from the remaining components
        if let Some(Component::Prefix(_)) = components.next() {
            if let Some(Component::RootDir) = components.next() {
                let remainder: PathBuf = components.collect();
                return a.join(remainder);
            }
        }
        
        // Fallback: if we can't recognize the structure, just remove the drive letter
        let drive_letter_removed = b.to_string_lossy()
            .trim_start_matches(|c: char| c.is_ascii_alphabetic() || c == ':' || c == '\\')
            .to_string();
            
        return a.join(drive_letter_removed);
    }
    
    // Check if b starts with a using strip_prefix
    if let Ok(remaining) = b.strip_prefix(a) {
        return a.join(remaining);
    }
    
    // Handle the case where string representation matches but Path doesn't
    // (can happen with case differences or different path separators)
    let a_str = a.to_string_lossy();
    let b_str = b.to_string_lossy();
    
    // Convert Cow to &str before using starts_with
    if b_str.starts_with(a_str.as_ref()) {
        // Remove the prefix that matches a
        let remaining = &b_str[a_str.len()..];
        let remaining = remaining.trim_start_matches('\\');
        return a.join(remaining);
    }

    // Standard join for other cases
    a.join(b)
}

/// Creates a new symbolic link on the filesystem.
///
/// The `link` path will be a symbolic link pointing to the `original` path.
/// On Windows, we need to determine if the target is a file or directory.
pub(super) fn symlink_sync(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    // Determine if the original is a file or directory
    let meta = metadata(original.as_ref())?;
    if meta.is_dir() {
        std::os::windows::fs::symlink_dir(original, link)
    } else {
        std::os::windows::fs::symlink_file(original, link)
    }
}

/// Creates a new symbolic link asynchronously.
///
/// This is a helper function for the Windows implementation.
pub(super) async fn symlink_async(original: impl AsRef<Path>, link: impl AsRef<Path>) -> io::Result<()> {
    // Determine if the original is a file or directory
    let meta = metadata(original.as_ref())?;
    if meta.is_dir() {
        tokio::fs::symlink_dir(original, link).await
    } else {
        tokio::fs::symlink_file(original, link).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_append() {
        macro_rules! assert_append {
            ($a:expr, $b:expr, $expected:expr) => {
                assert_eq!(append($a, $b), PathBuf::from($expected));
            };
        }

        // Test different drive letters (should strip prefix)
        assert_append!("C:\\temp", "D:\\test", "C:\\temp\\test");
        
        // Test same path prefixes (should use strip_prefix)
        assert_append!("C:\\temp", "C:\\temp\\subdir", "C:\\temp\\subdir");
        assert_append!("C:\\temp", "C:\\temp\\subdir\\file.txt", "C:\\temp\\subdir\\file.txt");
        
        // Test relative path (standard join)
        assert_append!("C:\\temp", "subdir\\file.txt", "C:\\temp\\subdir\\file.txt");
        
        // Test different absolute paths with same drive (strip drive and root)
        assert_append!("C:\\temprootdir", "C:\\test_file.txt", "C:\\temprootdir\\test_file.txt");
        
        // Test different absolute paths with different drives
        assert_append!("C:\\temprootdir", "D:\\test_file.txt", "C:\\temprootdir\\test_file.txt");
        
        // Test paths with mixed case (should be case-insensitive on Windows)
        assert_append!("C:\\Temp", "c:\\temp\\file.txt", "C:\\Temp\\file.txt");
    }
}

#[cfg(test)]
#[cfg(windows)]
mod integration_tests {
    use super::*;
    use tempfile::TempDir;
    
    #[test]
    fn test_append_with_real_paths() {
        // Create a temporary directory for testing
        let temp_dir = TempDir::new().unwrap();
        let temp_path = temp_dir.path();
        
        // Test appending an absolute path
        let drive_letter = temp_path.to_string_lossy().chars().next().unwrap_or('C');
        let absolute_path = format!("{}:\\test.txt", drive_letter);
        
        let result = append(temp_path, absolute_path);
        assert!(result.to_string_lossy().contains("test.txt"));
        assert!(!result.to_string_lossy().contains(":\\test.txt"));
    }
}
