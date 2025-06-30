use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use eyre::{Result, eyre};
use glob::glob;
use regex::Regex;
use serde::{Deserialize, Serialize};


use super::consts::CONTEXT_FILES_MAX_SIZE;
use super::tools::execute::dangerous_patterns;
use super::util::drop_matched_context_files;
use crate::cli::chat::ChatError;
use crate::cli::chat::cli::hooks::{Hook, HookExecutor};
use crate::os::Os;
use crate::util::directories;

pub const AMAZONQ_FILENAME: &str = "AmazonQ.md";

/// Represents a trusted command pattern that can be executed without user confirmation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TrustedCommand {
    /// The command pattern using glob-style matching (with * wildcards).
    /// Examples: "npm *", "git status", "git restore *"
    pub command: String,

    /// Optional description for documentation purposes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Configuration for trusted commands that can be executed without user confirmation.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct TrustedCommandsConfig {
    /// List of trusted command patterns.
    pub trusted_commands: Vec<TrustedCommand>,
}

/// Processed trusted commands for efficient pattern matching.
#[derive(Debug, Clone)]
pub struct ProcessedTrustedCommands {
    /// List of command patterns with their descriptions.
    patterns: Vec<(String, Option<String>)>,
}

impl ProcessedTrustedCommands {
    /// Create a new ProcessedTrustedCommands from a TrustedCommandsConfig.
    pub fn new(config: TrustedCommandsConfig) -> Self {
        let patterns = config
            .trusted_commands
            .into_iter()
            .map(|cmd| (cmd.command, cmd.description))
            .collect();

        Self { patterns }
    }

    /// Check if a command is trusted by matching against the patterns.
    pub fn is_trusted(&self, command: &str) -> bool {
        self.patterns
            .iter()
            .any(|(pattern, _)| Self::glob_match(pattern, command))
    }

    /// Perform glob-style pattern matching with * wildcards.
    /// Returns true if the pattern matches the command.
    fn glob_match(pattern: &str, command: &str) -> bool {
        // Handle exact match first
        if pattern == command {
            return true;
        }

        // Convert glob pattern to regex
        let regex_pattern = pattern
            .replace("*", ".*") // Replace * with .*
            .replace("?", "."); // Replace ? with . (single character)

        // Add anchors to match the entire string
        let regex_pattern = format!("^{}$", regex_pattern);

        // Compile and match
        if let Ok(regex) = Regex::new(&regex_pattern) {
            regex.is_match(command)
        } else {
            // If regex compilation fails, fall back to exact match
            pattern == command
        }
    }
}

/// Configuration for context files, containing paths to include in the context.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ContextConfig {
    /// List of file paths or glob patterns to include in the context.
    pub paths: Vec<String>,

    /// Map of Hook Name to [`Hook`]. The hook name serves as the hook's ID.
    pub hooks: HashMap<String, Hook>,

    /// Trusted commands configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trusted_commands: Option<TrustedCommandsConfig>,
}

/// Manager for context files and profiles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextManager {
    max_context_files_size: usize,

    /// Global context configuration that applies to all profiles.
    pub global_config: ContextConfig,

    /// Name of the current active profile.
    pub current_profile: String,

    /// Context configuration for the current profile.
    pub profile_config: ContextConfig,

    #[serde(skip)]
    pub hook_executor: HookExecutor,
}

impl ContextManager {
    /// Create a new ContextManager with default settings.
    pub async fn new(os: &Os, max_context_files_size: Option<usize>) -> Result<Self> {
        let max_context_files_size = max_context_files_size.unwrap_or(CONTEXT_FILES_MAX_SIZE);
        let profiles_dir = directories::chat_profiles_dir(os)?;
        os.fs.create_dir_all(&profiles_dir).await?;
        let global_config = load_global_config(os).await?;
        let current_profile = "default".to_string();
        let profile_config = load_profile_config(os, &current_profile).await?;

        Ok(Self {
            max_context_files_size,
            global_config,
            current_profile,
            profile_config,
            hook_executor: HookExecutor::new(),
        })
    }

    async fn save_config(&self, os: &Os, global: bool) -> Result<()> {
        if global {
            let global_path = directories::chat_global_context_path(os)?;
            let contents = serde_json::to_string_pretty(&self.global_config)
                .map_err(|e| eyre!("Failed to serialize global configuration: {}", e))?;
            os.fs.write(&global_path, contents).await?;
        } else {
            let profile_path = profile_context_path(os, &self.current_profile)?;
            if let Some(parent) = profile_path.parent() {
                os.fs.create_dir_all(parent).await?;
            }
            let contents = serde_json::to_string_pretty(&self.profile_config)
                .map_err(|e| eyre!("Failed to serialize profile configuration: {}", e))?;
            os.fs.write(&profile_path, contents).await?;
        }
        Ok(())
    }

    pub async fn reload_config(&mut self, os: &Os) -> Result<()> {
        self.global_config = load_global_config(os).await?;
        self.profile_config = load_profile_config(os, &self.current_profile).await?;
        Ok(())
    }

    pub async fn add_paths(&mut self, os: &Os, paths: Vec<String>, global: bool, force: bool) -> Result<()> {
        let mut all_paths = self.global_config.paths.clone();
        all_paths.append(&mut self.profile_config.paths.clone());

        // Validate paths exist before adding them
        if !force {
            let mut context_files = Vec::new();
            for path in &paths {
                match process_path(os, path, &mut context_files, true).await {
                    Ok(_) => {},
                    Err(e) => return Err(eyre!("Invalid path '{}': {}. Use --force to add anyway.", path, e)),
                }
            }
        }

        // Add each path, checking for duplicates
        for path in paths {
            if all_paths.contains(&path) {
                return Err(eyre!("Rule '{}' already exists.", path));
            }
            if global {
                self.global_config.paths.push(path);
            } else {
                self.profile_config.paths.push(path);
            }
        }

        self.save_config(os, global).await?;
        Ok(())
    }

    pub async fn remove_paths(&mut self, os: &Os, paths: Vec<String>, global: bool) -> Result<()> {
        let config = self.get_config_mut(global);
        let mut removed_any = false;

        for path in paths {
            let original_len = config.paths.len();
            config.paths.retain(|p| p != &path);
            if config.paths.len() < original_len {
                removed_any = true;
            }
        }

        if !removed_any {
            return Err(eyre!("None of the specified paths were found in the context"));
        }

        self.save_config(os, global).await?;
        Ok(())
    }

    pub async fn clear(&mut self, os: &Os, global: bool) -> Result<()> {
        if global {
            self.global_config.paths.clear();
        } else {
            self.profile_config.paths.clear();
        }
        self.save_config(os, global).await?;
        Ok(())
    }

    pub async fn list_profiles(&self, os: &Os) -> Result<Vec<String>> {
        let mut profiles = Vec::new();
        profiles.push("default".to_string());

        let profiles_dir = directories::chat_profiles_dir(os)?;
        if profiles_dir.exists() {
            let mut read_dir = os.fs.read_dir(&profiles_dir).await?;
            while let Some(entry) = read_dir.next_entry().await? {
                let path = entry.path();
                if let (true, Some(name)) = (path.is_dir(), path.file_name()) {
                    if name != "default" {
                        profiles.push(name.to_string_lossy().to_string());
                    }
                }
            }
        }

        if profiles.len() > 1 {
            profiles[1..].sort();
        }
        Ok(profiles)
    }

    pub fn list_profiles_blocking(&self, os: &Os) -> Result<Vec<String>> {
        let mut profiles = Vec::new();
        profiles.push("default".to_string());

        let profiles_dir = directories::chat_profiles_dir(os)?;
        if profiles_dir.exists() {
            for entry in std::fs::read_dir(profiles_dir)? {
                let entry = entry?;
                let path = entry.path();
                if let (true, Some(name)) = (path.is_dir(), path.file_name()) {
                    if name != "default" {
                        profiles.push(name.to_string_lossy().to_string());
                    }
                }
            }
        }

        if profiles.len() > 1 {
            profiles[1..].sort();
        }
        Ok(profiles)
    }

    pub async fn create_profile(&self, os: &Os, name: &str) -> Result<()> {
        validate_profile_name(name)?;

        let profile_path = profile_context_path(os, name)?;
        if profile_path.exists() {
            return Err(eyre!("Profile '{}' already exists", name));
        }

        let config = ContextConfig::default();
        let contents = serde_json::to_string_pretty(&config)
            .map_err(|e| eyre!("Failed to serialize profile configuration: {}", e))?;

        if let Some(parent) = profile_path.parent() {
            os.fs.create_dir_all(parent).await?;
        }
        os.fs.write(&profile_path, contents).await?;
        Ok(())
    }

    pub async fn delete_profile(&self, os: &Os, name: &str) -> Result<()> {
        if name == "default" {
            return Err(eyre!("Cannot delete the default profile"));
        } else if name == self.current_profile {
            return Err(eyre!(
                "Cannot delete the active profile. Switch to another profile first"
            ));
        }

        let profile_path = profile_dir_path(os, name)?;
        if !profile_path.exists() {
            return Err(eyre!("Profile '{}' does not exist", name));
        }

        os.fs.remove_dir_all(&profile_path).await?;
        Ok(())
    }

    pub async fn switch_profile(&mut self, os: &Os, name: &str) -> Result<()> {
        validate_profile_name(name)?;
        self.hook_executor.profile_cache.clear();

        if name == "default" {
            let profile_config = load_profile_config(os, name).await?;
            self.current_profile = name.to_string();
            self.profile_config = profile_config;
            return Ok(());
        }

        let profile_path = profile_context_path(os, name)?;
        if !profile_path.exists() {
            return Err(eyre!("Profile '{}' does not exist. Use 'create' to create it", name));
        }

        self.current_profile = name.to_string();
        self.profile_config = load_profile_config(os, name).await?;
        Ok(())
    }

    pub async fn rename_profile(&mut self, os: &Os, old_name: &str, new_name: &str) -> Result<()> {
        if old_name == "default" {
            return Err(eyre!("Cannot rename the default profile"));
        }
        if new_name == "default" {
            return Err(eyre!("Cannot rename to 'default' as it's a reserved profile name"));
        }

        validate_profile_name(new_name)?;

        let old_profile_path = profile_dir_path(os, old_name)?;
        if !old_profile_path.exists() {
            return Err(eyre!("Profile '{}' not found", old_name));
        }

        let new_profile_path = profile_dir_path(os, new_name)?;
        if new_profile_path.exists() {
            return Err(eyre!("Profile '{}' already exists", new_name));
        }

        os.fs.rename(&old_profile_path, &new_profile_path).await?;

        if self.current_profile == old_name {
            self.current_profile = new_name.to_string();
            self.profile_config = load_profile_config(os, new_name).await?;
        }

        Ok(())
    }

    pub async fn get_context_files(&self, os: &Os) -> Result<Vec<(String, String)>> {
        let mut context_files = Vec::new();

        self.collect_context_files(os, &self.global_config.paths, &mut context_files)
            .await?;
        self.collect_context_files(os, &self.profile_config.paths, &mut context_files)
            .await?;

        context_files.sort_by(|a, b| a.0.cmp(&b.0));
        context_files.dedup_by(|a, b| a.0 == b.0);

        Ok(context_files)
    }

    pub async fn get_context_files_by_path(&self, os: &Os, path: &str) -> Result<Vec<(String, String)>> {
        let mut context_files = Vec::new();
        process_path(os, path, &mut context_files, true).await?;
        Ok(context_files)
    }

    pub async fn collect_context_files_with_limit(
        &self,
        os: &Os,
    ) -> Result<(Vec<(String, String)>, Vec<(String, String)>)> {
        let mut files = self.get_context_files(os).await?;
        let dropped_files = drop_matched_context_files(&mut files, self.max_context_files_size).unwrap_or_default();
        files.retain(|file| !dropped_files.iter().any(|dropped| dropped.0 == file.0));
        Ok((files, dropped_files))
    }

    async fn collect_context_files(
        &self,
        os: &Os,
        paths: &[String],
        context_files: &mut Vec<(String, String)>,
    ) -> Result<()> {
        for path in paths {
            process_path(os, path, context_files, false).await?;
        }
        Ok(())
    }

    pub async fn add_hook(&mut self, os: &Os, name: String, hook: Hook, global: bool) -> Result<()> {
        let config = self.get_config_mut(global);

        if config.hooks.contains_key(&name) {
            return Err(eyre!("name already exists."));
        }

        config.hooks.insert(name, hook);
        self.save_config(os, global).await
    }

    pub async fn remove_hook(&mut self, os: &Os, name: &str, global: bool) -> Result<()> {
        let config = self.get_config_mut(global);

        if !config.hooks.contains_key(name) {
            return Err(eyre!("does not exist."));
        }

        config.hooks.remove(name);
        self.save_config(os, global).await
    }

    pub async fn set_hook_disabled(&mut self, os: &Os, name: &str, global: bool, disable: bool) -> Result<()> {
        let config = self.get_config_mut(global);

        if !config.hooks.contains_key(name) {
            return Err(eyre!("does not exist."));
        }

        if let Some(hook) = config.hooks.get_mut(name) {
            hook.disabled = disable;
        }

        self.save_config(os, global).await
    }

    pub async fn set_all_hooks_disabled(&mut self, os: &Os, global: bool, disable: bool) -> Result<()> {
        let config = self.get_config_mut(global);
        config.hooks.iter_mut().for_each(|(_, h)| h.disabled = disable);
        self.save_config(os, global).await
    }

    pub async fn run_hooks(&mut self, output: &mut impl Write) -> Result<Vec<(Hook, String)>, ChatError> {
        let mut hooks: Vec<&Hook> = Vec::new();

        let configs = [
            (&mut self.global_config.hooks, true),
            (&mut self.profile_config.hooks, false),
        ];

        for (hook_list, is_global) in configs {
            hooks.extend(hook_list.iter_mut().map(|(name, h)| {
                h.name = name.clone();
                h.is_global = is_global;
                &*h
            }));
        }

        self.hook_executor.run_hooks(hooks, output).await
    }

    pub async fn add_trusted_command(&mut self, os: &Os, trusted_command: TrustedCommand, global: bool) -> Result<()> {
        self.validate_trusted_command(&trusted_command)?;

        let config = self.get_config_mut(global);

        if config.trusted_commands.is_none() {
            config.trusted_commands = Some(TrustedCommandsConfig::default());
        }

        if let Some(ref mut trusted_commands_config) = config.trusted_commands {
            if let Some(existing_cmd) = trusted_commands_config
                .trusted_commands
                .iter_mut()
                .find(|cmd| cmd.command == trusted_command.command)
            {
                existing_cmd.description = trusted_command.description.clone();
                self.save_config(os, global)
                    .await
                    .map_err(|e| eyre!("Failed to update trusted command '{}': {}", trusted_command.command, e))?;

                tracing::info!(
                    "Updated description for trusted command pattern '{}' in {} configuration",
                    trusted_command.command,
                    if global { "global" } else { "profile" }
                );
                return Ok(());
            }
        }

        config
            .trusted_commands
            .as_mut()
            .unwrap()
            .trusted_commands
            .push(trusted_command.clone());

        self.save_config(os, global)
            .await
            .map_err(|e| eyre!("Failed to save trusted command '{}': {}", trusted_command.command, e))?;

        tracing::info!(
            "Added new trusted command pattern '{}' to {} configuration",
            trusted_command.command,
            if global { "global" } else { "profile" }
        );
        Ok(())
    }

    fn validate_trusted_command(&self, trusted_command: &TrustedCommand) -> Result<()> {
        if trusted_command.command.trim().is_empty() {
            return Err(eyre!("Command pattern cannot be empty"));
        }

        if let Some(pattern_match) = dangerous_patterns::check_all_dangerous_patterns(&trusted_command.command) {
            let reason = match pattern_match.pattern_type {
                dangerous_patterns::DangerousPatternType::Destructive => "destructive command",
                dangerous_patterns::DangerousPatternType::ShellControl => "shell control pattern",
                dangerous_patterns::DangerousPatternType::IoRedirection => "I/O redirection pattern",
            };
            return Err(eyre!(
                "Command pattern '{}' contains dangerous pattern '{}' ({}) and cannot be trusted",
                trusted_command.command,
                pattern_match.pattern,
                reason
            ));
        }

        let regex_pattern = trusted_command.command.replace("*", ".*").replace("?", ".");
        let regex_pattern = format!("^{}$", regex_pattern);

        if regex::Regex::new(&regex_pattern).is_err() {
            return Err(eyre!(
                "Command pattern '{}' contains invalid regex syntax",
                trusted_command.command
            ));
        }

        Ok(())
    }

    pub fn get_trusted_commands(&self, global: bool) -> TrustedCommandsConfig {
        let config = if global {
            &self.global_config
        } else {
            &self.profile_config
        };

        config.trusted_commands.as_ref().cloned().unwrap_or_default()
    }

    pub fn get_combined_trusted_commands(&self) -> TrustedCommandsConfig {
        let mut combined = TrustedCommandsConfig::default();

        if let Some(ref global_trusted) = self.global_config.trusted_commands {
            combined
                .trusted_commands
                .extend(global_trusted.trusted_commands.clone());
        }

        if let Some(ref profile_trusted) = self.profile_config.trusted_commands {
            for cmd in &profile_trusted.trusted_commands {
                if !combined
                    .trusted_commands
                    .iter()
                    .any(|existing| existing.command == cmd.command)
                {
                    combined.trusted_commands.push(cmd.clone());
                }
            }
        }

        combined
    }

    pub fn get_processed_trusted_commands(&self) -> ProcessedTrustedCommands {
        let combined_config = self.get_combined_trusted_commands();
        ProcessedTrustedCommands::new(combined_config)
    }

    pub async fn remove_trusted_command(&mut self, os: &Os, command_pattern: &str, global: bool) -> Result<()> {
        let config = self.get_config_mut(global);

        if let Some(ref mut trusted_commands_config) = config.trusted_commands {
            let original_len = trusted_commands_config.trusted_commands.len();
            trusted_commands_config
                .trusted_commands
                .retain(|cmd| cmd.command != command_pattern);

            if trusted_commands_config.trusted_commands.len() < original_len {
                self.save_config(os, global).await?;
                Ok(())
            } else {
                Err(eyre!("Trusted command pattern '{}' not found", command_pattern))
            }
        } else {
            Err(eyre!("No trusted commands configuration found"))
        }
    }

    pub async fn clear_trusted_commands(&mut self, os: &Os, global: bool) -> Result<()> {
        let config = self.get_config_mut(global);

        if let Some(ref mut trusted_commands_config) = config.trusted_commands {
            trusted_commands_config.trusted_commands.clear();
        } else {
            config.trusted_commands = Some(TrustedCommandsConfig::default());
        }

        self.save_config(os, global).await?;
        Ok(())
    }

    fn get_config_mut(&mut self, global: bool) -> &mut ContextConfig {
        if global {
            &mut self.global_config
        } else {
            &mut self.profile_config
        }
    }
}

fn profile_dir_path(os: &Os, profile_name: &str) -> Result<PathBuf> {
    Ok(directories::chat_profiles_dir(os)?.join(profile_name))
}

pub fn profile_context_path(os: &Os, profile_name: &str) -> Result<PathBuf> {
    Ok(directories::chat_profiles_dir(os)?
        .join(profile_name)
        .join("context.json"))
}

async fn load_global_config(os: &Os) -> Result<ContextConfig> {
    let global_path = directories::chat_global_context_path(os)?;
    if os.fs.exists(&global_path) {
        let contents = os.fs.read_to_string(&global_path).await?;
        let config: ContextConfig =
            serde_json::from_str(&contents).map_err(|e| eyre!("Failed to parse global configuration: {}", e))?;
        Ok(config)
    } else {
        Ok(get_default_global_config())
    }
}

fn get_default_global_config() -> ContextConfig {
    ContextConfig {
        paths: vec![
            ".amazonq/rules/**/*.md".to_string(),
            "README.md".to_string(),
            AMAZONQ_FILENAME.to_string(),
        ],
        hooks: HashMap::new(),
        trusted_commands: None,
    }
}

async fn load_profile_config(os: &Os, profile_name: &str) -> Result<ContextConfig> {
    let profile_path = profile_context_path(os, profile_name)?;
    if os.fs.exists(&profile_path) {
        let contents = os.fs.read_to_string(&profile_path).await?;
        let config: ContextConfig =
            serde_json::from_str(&contents).map_err(|e| eyre!("Failed to parse profile configuration: {}", e))?;
        Ok(config)
    } else {
        Ok(ContextConfig::default())
    }
}

async fn process_path(
    os: &Os,
    path: &str,
    context_files: &mut Vec<(String, String)>,
    is_validation: bool,
) -> Result<()> {
    let expanded_path = if path.starts_with('~') {
        let home = os.env.home().unwrap_or_default();
        path.replacen('~', &home.to_string_lossy(), 1)
    } else {
        path.to_string()
    };

    if expanded_path.contains('*') || expanded_path.contains('?') || expanded_path.contains('[') {
        let glob_results = glob(&expanded_path)?;
        let mut found_any = false;

        for entry in glob_results {
            match entry {
                Ok(path) => {
                    found_any = true;
                    add_file_to_context(os, &path, context_files).await?;
                },
                Err(e) => {
                    if is_validation {
                        return Err(eyre!("Glob pattern error: {}", e));
                    }
                },
            }
        }

        if is_validation && !found_any {
            return Err(eyre!("Glob pattern '{}' did not match any files", expanded_path));
        }
    } else {
        let path = PathBuf::from(&expanded_path);
        if os.fs.exists(&path) {
            add_file_to_context(os, &path, context_files).await?;
        } else if is_validation {
            return Err(eyre!("Path '{}' does not exist", expanded_path));
        }
    }

    Ok(())
}

async fn add_file_to_context(os: &Os, path: &Path, context_files: &mut Vec<(String, String)>) -> Result<()> {
    // Use os.fs to check if it's a file since we're in a test environment
    let metadata = match os.fs.symlink_metadata(path).await {
        Ok(metadata) => metadata,
        Err(_e) => {
            return Ok(());
        }
    };
    
    if metadata.is_file() {
        match os.fs.read_to_string(path).await {
            Ok(content) => {
                let filename = path.to_string_lossy().to_string();

                context_files.push((filename, content));
            },
            Err(e) => {
                eprintln!("Failed to read file '{}': {}", path.display(), e);
                tracing::warn!("Failed to read file '{}': {}", path.display(), e);
            },
        }
    } else if metadata.is_dir() {
        // For directories, only read direct files (non-recursive to avoid the boxing issue)
        let mut read_dir = os.fs.read_dir(path).await?;
        while let Some(entry) = read_dir.next_entry().await? {
            let entry_path = entry.path();
            if entry_path.is_file() {
                match os.fs.read_to_string(&entry_path).await {
                    Ok(content) => {
                        let filename = entry_path.to_string_lossy().to_string();
                        context_files.push((filename, content));
                    },
                    Err(e) => {
                        tracing::warn!("Failed to read file '{}': {}", entry_path.display(), e);
                    },
                }
            }
        }
    }

    Ok(())
}

fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(eyre!("Profile name cannot be empty"));
    }

    if !name.chars().next().unwrap().is_alphanumeric() {
        return Err(eyre!("Profile name must start with an alphanumeric character"));
    }

    if !name.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return Err(eyre!(
            "Profile name can only contain alphanumeric characters, hyphens, and underscores"
        ));
    }

    Ok(())
}
