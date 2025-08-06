use std::collections::HashMap;
use std::env;

use eyre::{
    Result,
    eyre,
};
use regex::Regex;

/// Expands environment variables in a string using the format ${VAR_NAME}
///
/// # Arguments
/// * `input` - The input string that may contain environment variable placeholders
///
/// # Returns
/// * `Result<String>` - The expanded string with environment variables substituted
///
/// # Examples
/// ```
/// use std::env;
///
/// env::set_var("TEST_VAR", "hello");
/// let result = expand_env_vars("Value is ${TEST_VAR}").unwrap();
/// assert_eq!(result, "Value is hello");
/// ```
pub fn expand_env_vars(input: &str) -> Result<String> {
    let re = Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}")?;
    let mut result = input.to_string();
    let mut missing_vars = Vec::new();

    for captures in re.captures_iter(input) {
        let full_match = captures.get(0).unwrap().as_str();
        let var_name = captures.get(1).unwrap().as_str();

        match env::var(var_name) {
            Ok(value) => {
                result = result.replace(full_match, &value);
            },
            Err(_) => {
                missing_vars.push(var_name.to_string());
            },
        }
    }

    if !missing_vars.is_empty() {
        return Err(eyre!("Environment variables not found: {}", missing_vars.join(", ")));
    }

    Ok(result)
}

/// Expands environment variables in a HashMap of environment variables
///
/// # Arguments
/// * `env_vars` - HashMap containing environment variable key-value pairs
///
/// # Returns
/// * `Result<HashMap<String, String>>` - HashMap with expanded environment variables
pub fn expand_env_vars_in_map(env_vars: &HashMap<String, String>) -> Result<HashMap<String, String>> {
    let mut expanded = HashMap::new();

    for (key, value) in env_vars {
        let expanded_value = expand_env_vars(value)?;
        expanded.insert(key.clone(), expanded_value);
    }

    Ok(expanded)
}

/// Expands environment variables in command arguments
///
/// # Arguments
/// * `args` - Vector of command arguments that may contain environment variable placeholders
///
/// # Returns
/// * `Result<Vec<String>>` - Vector with expanded command arguments
pub fn expand_env_vars_in_args(args: &[String]) -> Result<Vec<String>> {
    let mut expanded = Vec::new();

    for arg in args {
        let expanded_arg = expand_env_vars(arg)?;
        expanded.push(expanded_arg);
    }

    Ok(expanded)
}

/// Expands environment variables in a command string
///
/// # Arguments
/// * `command` - Command string that may contain environment variable placeholders
///
/// # Returns
/// * `Result<String>` - Expanded command string
pub fn expand_env_vars_in_command(command: &str) -> Result<String> {
    expand_env_vars(command)
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::*;

    #[test]
    fn test_expand_env_vars_simple() {
        env::set_var("TEST_VAR", "hello");
        let result = expand_env_vars("Value is ${TEST_VAR}").unwrap();
        assert_eq!(result, "Value is hello");
        env::remove_var("TEST_VAR");
    }

    #[test]
    fn test_expand_env_vars_multiple() {
        env::set_var("VAR1", "hello");
        env::set_var("VAR2", "world");
        let result = expand_env_vars("${VAR1} ${VAR2}!").unwrap();
        assert_eq!(result, "hello world!");
        env::remove_var("VAR1");
        env::remove_var("VAR2");
    }

    #[test]
    fn test_expand_env_vars_no_vars() {
        let result = expand_env_vars("no variables here").unwrap();
        assert_eq!(result, "no variables here");
    }

    #[test]
    fn test_expand_env_vars_missing_var() {
        let result = expand_env_vars("Value is ${NONEXISTENT_VAR}");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("NONEXISTENT_VAR"));
    }

    #[test]
    fn test_expand_env_vars_in_map() {
        env::set_var("USERNAME", "testuser");
        env::set_var("PASSWORD", "testpass");

        let mut env_vars = HashMap::new();
        env_vars.insert("USER".to_string(), "${USERNAME}".to_string());
        env_vars.insert("PASS".to_string(), "${PASSWORD}".to_string());
        env_vars.insert("STATIC".to_string(), "static_value".to_string());

        let result = expand_env_vars_in_map(&env_vars).unwrap();

        assert_eq!(result.get("USER").unwrap(), "testuser");
        assert_eq!(result.get("PASS").unwrap(), "testpass");
        assert_eq!(result.get("STATIC").unwrap(), "static_value");

        env::remove_var("USERNAME");
        env::remove_var("PASSWORD");
    }

    #[test]
    fn test_expand_env_vars_in_args() {
        env::set_var("ARG_VAR", "expanded_arg");

        let args = vec![
            "--config".to_string(),
            "${ARG_VAR}".to_string(),
            "static_arg".to_string(),
        ];

        let result = expand_env_vars_in_args(&args).unwrap();

        assert_eq!(result[0], "--config");
        assert_eq!(result[1], "expanded_arg");
        assert_eq!(result[2], "static_arg");

        env::remove_var("ARG_VAR");
    }

    #[test]
    fn test_expand_env_vars_in_command() {
        env::set_var("CMD_VAR", "python");

        let command = "${CMD_VAR}";
        let result = expand_env_vars_in_command(command).unwrap();

        assert_eq!(result, "python");

        env::remove_var("CMD_VAR");
    }

    #[test]
    fn test_complex_expansion() {
        env::set_var("HOME", "/home/user");
        env::set_var("APP_NAME", "myapp");

        let input = "${HOME}/.config/${APP_NAME}/config.json";
        let result = expand_env_vars(input).unwrap();

        assert_eq!(result, "/home/user/.config/myapp/config.json");

        env::remove_var("HOME");
        env::remove_var("APP_NAME");
    }
}
