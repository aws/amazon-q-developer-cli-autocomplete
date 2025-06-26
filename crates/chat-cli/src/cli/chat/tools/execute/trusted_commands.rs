// This module contains utilities for trusted command creation.
// The main implementation is in mod.rs as part of ChatSession.

#[cfg(test)]
mod tests {
    use super::super::super::super::ChatSession;

    #[test]
    fn test_generate_pattern_options_simple_command() {
        let options = ChatSession::generate_pattern_options("cat file.txt");
        assert_eq!(options.len(), 2); // Exact + first word (deduped)
        assert_eq!(options[0].0, "cat file.txt");
        assert_eq!(options[1].0, "cat*"); // First word only
    }

    #[test]
    fn test_generate_pattern_options_git_command() {
        let options = ChatSession::generate_pattern_options("git restore --staged Makefile frontend/ opentofu/");
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].0, "git restore --staged Makefile frontend/ opentofu/");
        assert_eq!(options[1].0, "git restore*");
        assert_eq!(options[2].0, "git*");
    }

    #[test]
    fn test_generate_pattern_options_npm_command() {
        let options = ChatSession::generate_pattern_options("npm run build");
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].0, "npm run build");
        assert_eq!(options[1].0, "npm run*");
        assert_eq!(options[2].0, "npm*"); // First word only
    }

    #[test]
    fn test_generate_pattern_options_single_word() {
        let options = ChatSession::generate_pattern_options("pwd");
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].0, "pwd");
        assert_eq!(options[0].1, "Trust this exact command only");
    }

    #[test]
    fn test_generate_pattern_options_command_with_flags_only() {
        let options = ChatSession::generate_pattern_options("ls -la");
        assert_eq!(options.len(), 2); // Exact + first word (stops at "-la")
        assert_eq!(options[0].0, "ls -la");
        assert_eq!(options[1].0, "ls*"); // First word only (nothing before "-")
    }

    #[test]
    fn test_generate_pattern_options_no_duplicate_patterns() {
        // Test case where --version is a flag, not a subcommand
        let options = ChatSession::generate_pattern_options("docker --version");
        assert_eq!(options.len(), 2); // Exact + first word (stops at "--version")
        assert_eq!(options[0].0, "docker --version");
        assert_eq!(options[1].0, "docker*"); // First word only (nothing before "--")
    }

    #[test]
    fn test_generate_pattern_options_multiple_words_before_flag() {
        // Test case with multiple words before hitting a flag
        let options = ChatSession::generate_pattern_options("git commit -m 'my message'");
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].0, "git commit -m 'my message'"); // Exact
        assert_eq!(options[1].0, "git commit*"); // Everything until "-m"
        assert_eq!(options[2].0, "git*"); // First word only
    }

    #[test]
    fn test_generate_pattern_options_no_flags() {
        // Test case with multiple words but no flags
        let options = ChatSession::generate_pattern_options("rsync source dest backup");
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].0, "rsync source dest backup"); // Exact
        assert_eq!(options[1].0, "rsync source*"); // Everything until "-" (no "-" found, so all args + *)
        assert_eq!(options[2].0, "rsync*"); // First word only
    }
}