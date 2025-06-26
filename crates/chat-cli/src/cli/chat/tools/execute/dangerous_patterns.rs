/// Centralized dangerous patterns for command validation
/// 
/// This module defines dangerous command patterns that should be treated with caution
/// across the entire application to maintain consistency and security.

/// Shell redirection and control patterns that can be dangerous
pub const SHELL_CONTROL_PATTERNS: &[&str] = &[
    "<(",     // Process substitution
    "$(",     // Command substitution  
    "`",      // Command substitution (backticks)
    ">",      // Output redirection
    ">>",     // Append redirection
    "&&",     // Logical AND
    "||",     // Logical OR
    "&",      // Background execution
    ";",      // Command separator
    "|",      // Pipe (handled separately in some contexts)
];

/// Destructive command patterns that should never be trusted
pub const DESTRUCTIVE_COMMAND_PATTERNS: &[&str] = &[
    "rm -rf",           // Recursive force remove
    "sudo rm",          // Privileged remove
    "format",           // Format disk
    "mkfs",             // Make filesystem
    "dd if=",           // Disk dump
    ":(){ :|:& };:",    // Fork bomb
    "> /dev/",          // Write to device files
    "chmod 777",        // Dangerous permissions
    "chown root",       // Change ownership to root
    "su -",             // Switch user
    "sudo su",          // Privileged user switch
    "del /",            // Windows delete (recursive)
    "rmdir /s",         // Windows remove directory
];

/// I/O redirection patterns that can be misused
pub const IO_REDIRECTION_PATTERNS: &[&str] = &[
    "> /dev/null",      // Redirect to null
    "2>&1",             // Redirect stderr to stdout
    "&>",               // Redirect both stdout and stderr
];



/// Represents the type of dangerous pattern found
#[derive(Debug, Clone, PartialEq)]
pub enum DangerousPatternType {
    /// Shell control patterns that affect execution safety
    ShellControl,
    /// Destructive command patterns that should never be trusted
    Destructive,
    /// I/O redirection patterns that can be misused
    IoRedirection,
}

/// Result of checking for dangerous patterns
#[derive(Debug, Clone, PartialEq)]
pub struct DangerousPatternMatch {
    /// The pattern that was matched
    pub pattern: &'static str,
    /// The type of dangerous pattern
    pub pattern_type: DangerousPatternType,
}

/// Comprehensive check for all types of dangerous patterns
/// 
/// This method checks for shell control, destructive, and I/O redirection patterns
/// and returns the first match found, prioritizing destructive patterns.
/// 
/// # Arguments
/// * `command` - The command string to check
/// 
/// # Returns
/// * `Some(DangerousPatternMatch)` if a dangerous pattern is found
/// * `None` if no dangerous patterns are found
/// 
/// # Priority Order
/// 1. Destructive patterns (highest priority - should never be trusted)
/// 2. Shell control patterns (medium priority - execution safety)
/// 3. I/O redirection patterns (lowest priority - can be misused)
pub fn check_all_dangerous_patterns(command: &str) -> Option<DangerousPatternMatch> {
    // Check destructive patterns first (highest priority)
    if let Some(pattern) = DESTRUCTIVE_COMMAND_PATTERNS.iter().find(|&&p| command.contains(p)) {
        return Some(DangerousPatternMatch {
            pattern: *pattern,
            pattern_type: DangerousPatternType::Destructive,
        });
    }
    
    // Check shell control patterns second
    if let Some(pattern) = SHELL_CONTROL_PATTERNS.iter().find(|&&p| command.contains(p)) {
        return Some(DangerousPatternMatch {
            pattern: *pattern,
            pattern_type: DangerousPatternType::ShellControl,
        });
    }
    
    // Check I/O redirection patterns last
    if let Some(pattern) = IO_REDIRECTION_PATTERNS.iter().find(|&&p| command.contains(p)) {
        return Some(DangerousPatternMatch {
            pattern: *pattern,
            pattern_type: DangerousPatternType::IoRedirection,
        });
    }
    
    None
}

#[cfg(test)]
mod tests {
    use super::*;



    #[test]
    fn test_check_all_dangerous_patterns() {
        // Test destructive patterns (highest priority)
        let result = check_all_dangerous_patterns("rm -rf /");
        assert!(result.is_some());
        let match_result = result.unwrap();
        assert_eq!(match_result.pattern, "rm -rf");
        assert_eq!(match_result.pattern_type, DangerousPatternType::Destructive);
        
        // Test shell control patterns
        let result = check_all_dangerous_patterns("echo $(whoami)");
        assert!(result.is_some());
        let match_result = result.unwrap();
        assert_eq!(match_result.pattern, "$(");
        assert_eq!(match_result.pattern_type, DangerousPatternType::ShellControl);
        
        // Note: I/O redirection patterns overlap with shell control patterns
        // Since shell control patterns are checked first, they take precedence
        // Test a command that would match I/O redirection but gets caught by shell control
        let result = check_all_dangerous_patterns("ls 2>&1");
        assert!(result.is_some());
        let match_result = result.unwrap();
        // This matches ">" from shell control patterns, not "2>&1" from I/O redirection
        assert_eq!(match_result.pattern, ">");
        assert_eq!(match_result.pattern_type, DangerousPatternType::ShellControl);
        
        // Test priority: destructive should take precedence over shell control
        let result = check_all_dangerous_patterns("rm -rf / && echo done");
        assert!(result.is_some());
        let match_result = result.unwrap();
        assert_eq!(match_result.pattern, "rm -rf");
        assert_eq!(match_result.pattern_type, DangerousPatternType::Destructive);
        
        // Test safe command
        let result = check_all_dangerous_patterns("git status");
        assert!(result.is_none());
    }

    #[test]
    fn test_pattern_type_priority() {
        // Command with both destructive and shell control patterns
        // Should prioritize destructive
        let result = check_all_dangerous_patterns("sudo rm file && echo done");
        assert!(result.is_some());
        let match_result = result.unwrap();
        assert_eq!(match_result.pattern_type, DangerousPatternType::Destructive);
        
        // Command with shell control and I/O redirection
        // Should prioritize shell control (since ">" is checked before "2>&1")
        let result = check_all_dangerous_patterns("echo test > file 2>&1");
        assert!(result.is_some());
        let match_result = result.unwrap();
        assert_eq!(match_result.pattern, ">");
        assert_eq!(match_result.pattern_type, DangerousPatternType::ShellControl);
    }
}