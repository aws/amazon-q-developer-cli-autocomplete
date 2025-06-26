use std::collections::HashMap;
use std::io::Write;
use std::path::{
    Path,
    PathBuf,
};

use eyre::{
    Result,
    eyre,
};
use glob::glob;
use regex::Regex;
use serde::{
    Deserialize,
    Serialize,
};
use tracing::debug;

use super::consts::CONTEXT_FILES_MAX_SIZE;
use super::tools::execute::dangerous_patterns;
use super::util::drop_matched_context_files;
use crate::cli::chat::ChatError;
use crate::cli::chat::cli::hooks::{
    Hook,
    HookExecutor,
};
use crate::platform::Context;
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
    ///
    /// This will:
    /// 1. Create the necessary directories if they don't exist
    /// 2. Load the global configuration
    /// 3. Load the default profile configuration
    ///
    /// # Arguments
    /// * `ctx` - The context to use
    /// * `max_context_files_size` - Optional maximum token size for context files. If not provided,
    ///   defaults to `CONTEXT_FILES_MAX_SIZE`.
    ///
    /// # Returns
    /// A Result containing the new ContextManager or an error
    pub async fn new(ctx: &Context, max_context_files_size: Option<usize>) -> Result<Self> {
        let max_context_files_size = max_context_files_size.unwrap_or(CONTEXT_FILES_MAX_SIZE);

        let profiles_dir = directories::chat_profiles_dir(ctx)?;

        ctx.fs.create_dir_all(&profiles_dir).await?;

        let global_config = load_global_config(ctx).await?;
        let current_profile = "default".to_string();
        let profile_config = load_profile_config(ctx, &current_profile).await?;

        Ok(Self {
            max_context_files_size,
            global_config,
            current_profile,
            profile_config,
            hook_executor: HookExecutor::new(),
        })
    }

    /// Save the current configuration to disk.
    ///
    /// # Arguments
    /// * `global` - If true, save the global configuration; otherwise, save the current profile
    ///   configuration
    ///
    /// # Returns
    /// A Result indicating success or an error
    async fn save_config(&self, ctx: &Context, global: bool) -> Result<()> {
        if global {
            let global_path = directories::chat_global_context_path(ctx)?;
            self.save_config_to_path(ctx, &global_path, &self.global_config, "global").await
        } else {
            let profile_path = profile_context_path(ctx, &self.current_profile)?;
            self.save_config_to_path(ctx, &profile_path, &self.profile_config, &format!("profile '{}'", self.current_profile)).await
        }
    }
    
    /// Save configuration to a specific path with comprehensive error handling.
    ///
    /// # Arguments
    /// * `config_path` - Path to save the configuration to
    /// * `config` - Configuration to save
    /// * `config_type` - Type of configuration for error messages
    ///
    /// # Returns
    /// A Result indicating success or an error
    async fn save_config_to_path(
        &self,
        ctx: &Context,
        config_path: &Path,
        config: &ContextConfig,
        config_type: &str,
    ) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = config_path.parent() {
            ctx.fs.create_dir_all(parent).await
                .map_err(|e| eyre!("Failed to create directory '{}' for {} configuration: {}", 
                                  parent.display(), config_type, e))?;
        }
        
        // Serialize configuration with error handling
        let contents = serde_json::to_string_pretty(config)
            .map_err(|e| eyre!("Failed to serialize {} configuration: {}", config_type, e))?;
        
        // Write to file with error handling
        ctx.fs.write(config_path, contents).await
            .map_err(|e| eyre!("Failed to write {} configuration to '{}': {}", 
                              config_type, config_path.display(), e))?;
        
        tracing::debug!("Successfully saved {} configuration to '{}'", config_type, config_path.display());
        Ok(())
    }

    /// Reloads the global and profile config from disk.
    /// Handles errors gracefully by falling back to default configurations when files are corrupted.
    pub async fn reload_config(&mut self, ctx: &Context) -> Result<()> {
        // Reload global config with error handling
        match load_global_config(ctx).await {
            Ok(config) => {
                self.global_config = config;
                tracing::debug!("Successfully reloaded global configuration");
            }
            Err(e) => {
                tracing::warn!("Failed to reload global configuration, keeping current: {}", e);
                // Keep the current global config instead of failing
            }
        }
        
        // Reload profile config with error handling
        match load_profile_config(ctx, &self.current_profile).await {
            Ok(config) => {
                self.profile_config = config;
                tracing::debug!("Successfully reloaded profile '{}' configuration", self.current_profile);
            }
            Err(e) => {
                tracing::warn!("Failed to reload profile '{}' configuration, keeping current: {}", self.current_profile, e);
                // Keep the current profile config instead of failing
            }
        }
        
        Ok(())
    }

    /// Add paths to the context configuration.
    ///
    /// # Arguments
    /// * `paths` - List of paths to add
    /// * `global` - If true, add to global configuration; otherwise, add to current profile
    ///   configuration
    /// * `force` - If true, skip validation that the path exists
    ///
    /// # Returns
    /// A Result indicating success or an error
    pub async fn add_paths(&mut self, ctx: &Context, paths: Vec<String>, global: bool, force: bool) -> Result<()> {
        let mut all_paths = self.global_config.paths.clone();
        all_paths.append(&mut self.profile_config.paths.clone());

        // Validate paths exist before adding them
        if !force {
            let mut context_files = Vec::new();

            // Check each path to make sure it exists or matches at least one file
            for path in &paths {
                // We're using a temporary context_files vector just for validation
                // Pass is_validation=true to ensure we error if glob patterns don't match any files
                match process_path(ctx, path, &mut context_files, true).await {
                    Ok(_) => {}, // Path is valid
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

        // Save the updated configuration
        self.save_config(ctx, global).await?;

        Ok(())
    }

    /// Remove paths from the context configuration.
    ///
    /// # Arguments
    /// * `paths` - List of paths to remove
    /// * `global` - If true, remove from global configuration; otherwise, remove from current
    ///   profile configuration
    ///
    /// # Returns
    /// A Result indicating success or an error
    pub async fn remove_paths(&mut self, ctx: &Context, paths: Vec<String>, global: bool) -> Result<()> {
        // Get reference to the appropriate config
        let config = self.get_config_mut(global);

        // Track if any paths were removed
        let mut removed_any = false;

        // Remove each path if it exists
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

        // Save the updated configuration
        self.save_config(ctx, global).await?;

        Ok(())
    }

    /// List all available profiles.
    ///
    /// # Returns
    /// A Result containing a vector of profile names, with "default" always first
    pub async fn list_profiles(&self, ctx: &Context) -> Result<Vec<String>> {
        let mut profiles = Vec::new();

        // Always include default profile
        profiles.push("default".to_string());

        // Read profile directory and extract profile names
        let profiles_dir = directories::chat_profiles_dir(ctx)?;
        if profiles_dir.exists() {
            let mut read_dir = ctx.fs.read_dir(&profiles_dir).await?;
            while let Some(entry) = read_dir.next_entry().await? {
                let path = entry.path();
                if let (true, Some(name)) = (path.is_dir(), path.file_name()) {
                    if name != "default" {
                        profiles.push(name.to_string_lossy().to_string());
                    }
                }
            }
        }

        // Sort non-default profiles alphabetically
        if profiles.len() > 1 {
            profiles[1..].sort();
        }

        Ok(profiles)
    }

    /// List all available profiles using blocking operations.
    ///
    /// Similar to list_profiles but uses synchronous filesystem operations.
    ///
    /// # Returns
    /// A Result containing a vector of profile names, with "default" always first
    pub fn list_profiles_blocking(&self, ctx: &Context) -> Result<Vec<String>> {
        let _ = self;

        let mut profiles = Vec::new();

        // Always include default profile
        profiles.push("default".to_string());

        // Read profile directory and extract profile names
        let profiles_dir = directories::chat_profiles_dir(ctx)?;
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

        // Sort non-default profiles alphabetically
        if profiles.len() > 1 {
            profiles[1..].sort();
        }

        Ok(profiles)
    }

    /// Clear all paths from the context configuration.
    ///
    /// # Arguments
    /// * `global` - If true, clear global configuration; otherwise, clear current profile
    ///   configuration
    ///
    /// # Returns
    /// A Result indicating success or an error
    pub async fn clear(&mut self, ctx: &Context, global: bool) -> Result<()> {
        // Clear the appropriate config
        if global {
            self.global_config.paths.clear();
        } else {
            self.profile_config.paths.clear();
        }

        // Save the updated configuration
        self.save_config(ctx, global).await?;

        Ok(())
    }

    /// Create a new profile.
    ///
    /// # Arguments
    /// * `name` - Name of the profile to create
    ///
    /// # Returns
    /// A Result indicating success or an error
    pub async fn create_profile(&self, ctx: &Context, name: &str) -> Result<()> {
        validate_profile_name(name)?;

        // Check if profile already exists
        let profile_path = profile_context_path(ctx, name)?;
        if profile_path.exists() {
            return Err(eyre!("Profile '{}' already exists", name));
        }

        // Create empty profile configuration
        let config = ContextConfig::default();
        let contents = serde_json::to_string_pretty(&config)
            .map_err(|e| eyre!("Failed to serialize profile configuration: {}", e))?;

        // Create the file
        if let Some(parent) = profile_path.parent() {
            ctx.fs.create_dir_all(parent).await?;
        }
        ctx.fs.write(&profile_path, contents).await?;

        Ok(())
    }

    /// Delete a profile.
    ///
    /// # Arguments
    /// * `name` - Name of the profile to delete
    ///
    /// # Returns
    /// A Result indicating success or an error
    pub async fn delete_profile(&self, ctx: &Context, name: &str) -> Result<()> {
        if name == "default" {
            return Err(eyre!("Cannot delete the default profile"));
        } else if name == self.current_profile {
            return Err(eyre!(
                "Cannot delete the active profile. Switch to another profile first"
            ));
        }

        let profile_path = profile_dir_path(ctx, name)?;
        if !profile_path.exists() {
            return Err(eyre!("Profile '{}' does not exist", name));
        }

        ctx.fs.remove_dir_all(&profile_path).await?;

        Ok(())
    }

    /// Rename a profile.
    ///
    /// # Arguments
    /// * `old_name` - Current name of the profile
    /// * `new_name` - New name for the profile
    ///
    /// # Returns
    /// A Result indicating success or an error
    pub async fn rename_profile(&mut self, ctx: &Context, old_name: &str, new_name: &str) -> Result<()> {
        // Validate profile names
        if old_name == "default" {
            return Err(eyre!("Cannot rename the default profile"));
        }
        if new_name == "default" {
            return Err(eyre!("Cannot rename to 'default' as it's a reserved profile name"));
        }

        validate_profile_name(new_name)?;

        let old_profile_path = profile_dir_path(ctx, old_name)?;
        if !old_profile_path.exists() {
            return Err(eyre!("Profile '{}' not found", old_name));
        }

        let new_profile_path = profile_dir_path(ctx, new_name)?;
        if new_profile_path.exists() {
            return Err(eyre!("Profile '{}' already exists", new_name));
        }

        ctx.fs.rename(&old_profile_path, &new_profile_path).await?;

        // If the current profile is being renamed, update the current_profile field
        if self.current_profile == old_name {
            self.current_profile = new_name.to_string();
            self.profile_config = load_profile_config(ctx, new_name).await?;
        }

        Ok(())
    }

    /// Switch to a different profile.
    ///
    /// # Arguments
    /// * `name` - Name of the profile to switch to
    ///
    /// # Returns
    /// A Result indicating success or an error
    pub async fn switch_profile(&mut self, ctx: &Context, name: &str) -> Result<()> {
        validate_profile_name(name)?;
        self.hook_executor.profile_cache.clear();

        // Special handling for default profile - it always exists
        if name == "default" {
            // Load the default profile configuration
            let profile_config = load_profile_config(ctx, name).await?;

            // Update the current profile
            self.current_profile = name.to_string();
            self.profile_config = profile_config;

            return Ok(());
        }

        // Check if profile exists
        let profile_path = profile_context_path(ctx, name)?;
        if !profile_path.exists() {
            return Err(eyre!("Profile '{}' does not exist. Use 'create' to create it", name));
        }

        // Update the current profile
        self.current_profile = name.to_string();
        self.profile_config = load_profile_config(ctx, name).await?;

        Ok(())
    }

    /// Get all context files (global + profile-specific).
    ///
    /// This method:
    /// 1. Processes all paths in the global and profile configurations
    /// 2. Expands glob patterns to include matching files
    /// 3. Reads the content of each file
    /// 4. Returns a vector of (filename, content) pairs
    ///
    ///
    /// # Returns
    /// A Result containing a vector of (filename, content) pairs or an error
    pub async fn get_context_files(&self, ctx: &Context) -> Result<Vec<(String, String)>> {
        let mut context_files = Vec::new();

        self.collect_context_files(ctx, &self.global_config.paths, &mut context_files)
            .await?;
        self.collect_context_files(ctx, &self.profile_config.paths, &mut context_files)
            .await?;

        context_files.sort_by(|a, b| a.0.cmp(&b.0));
        context_files.dedup_by(|a, b| a.0 == b.0);

        Ok(context_files)
    }

    pub async fn get_context_files_by_path(&self, ctx: &Context, path: &str) -> Result<Vec<(String, String)>> {
        let mut context_files = Vec::new();
        process_path(ctx, path, &mut context_files, true).await?;
        Ok(context_files)
    }

    /// Collects context files and optionally drops files if the total size exceeds the limit.
    /// Returns (files_to_use, dropped_files)
    pub async fn collect_context_files_with_limit(
        &self,
        ctx: &Context,
    ) -> Result<(Vec<(String, String)>, Vec<(String, String)>)> {
        let mut files = self.get_context_files(ctx).await?;

        let dropped_files = drop_matched_context_files(&mut files, self.max_context_files_size).unwrap_or_default();

        // remove dropped files from files
        files.retain(|file| !dropped_files.iter().any(|dropped| dropped.0 == file.0));

        Ok((files, dropped_files))
    }

    async fn collect_context_files(
        &self,
        ctx: &Context,
        paths: &[String],
        context_files: &mut Vec<(String, String)>,
    ) -> Result<()> {
        for path in paths {
            // Use is_validation=false to handle non-matching globs gracefully
            process_path(ctx, path, context_files, false).await?;
        }
        Ok(())
    }

    fn get_config_mut(&mut self, global: bool) -> &mut ContextConfig {
        if global {
            &mut self.global_config
        } else {
            &mut self.profile_config
        }
    }

    /// Add hooks to the context config. If another hook with the same name already exists, throw an
    /// error.
    ///
    /// # Arguments
    /// * `hook` - name of the hook to delete
    /// * `global` - If true, the add to the global config. If false, add to the current profile
    ///   config.
    /// * `conversation_start` - If true, add the hook to conversation_start. Otherwise, it will be
    ///   added to per_prompt.
    pub async fn add_hook(&mut self, ctx: &Context, name: String, hook: Hook, global: bool) -> Result<()> {
        let config = self.get_config_mut(global);

        if config.hooks.contains_key(&name) {
            return Err(eyre!("name already exists."));
        }

        config.hooks.insert(name, hook);
        self.save_config(ctx, global).await
    }

    /// Delete hook(s) by name
    /// # Arguments
    /// * `name` - name of the hook to delete
    /// * `global` - If true, the delete from the global config. If false, delete from the current
    ///   profile config
    pub async fn remove_hook(&mut self, ctx: &Context, name: &str, global: bool) -> Result<()> {
        let config = self.get_config_mut(global);

        if !config.hooks.contains_key(name) {
            return Err(eyre!("does not exist."));
        }

        config.hooks.remove(name);

        self.save_config(ctx, global).await
    }

    /// Sets the "disabled" field on any [`Hook`] with the given name
    /// # Arguments
    /// * `disable` - Set "disabled" field to this value
    pub async fn set_hook_disabled(&mut self, ctx: &Context, name: &str, global: bool, disable: bool) -> Result<()> {
        let config = self.get_config_mut(global);

        if !config.hooks.contains_key(name) {
            return Err(eyre!("does not exist."));
        }

        if let Some(hook) = config.hooks.get_mut(name) {
            hook.disabled = disable;
        }

        self.save_config(ctx, global).await
    }

    /// Sets the "disabled" field on all [`Hook`]s
    /// # Arguments
    /// * `disable` - Set all "disabled" fields to this value
    pub async fn set_all_hooks_disabled(&mut self, ctx: &Context, global: bool, disable: bool) -> Result<()> {
        let config = self.get_config_mut(global);

        config.hooks.iter_mut().for_each(|(_, h)| h.disabled = disable);

        self.save_config(ctx, global).await
    }

    /// Run all the currently enabled hooks from both the global and profile contexts.
    /// Skipped hooks (disabled) will not appear in the output.
    /// # Arguments
    /// * `updates` - output stream to write hook run status to if Some, else do nothing if None
    /// # Returns
    /// A vector containing pairs of a [`Hook`] definition and its execution output
    pub async fn run_hooks(&mut self, output: &mut impl Write) -> Result<Vec<(Hook, String)>, ChatError> {
        let mut hooks: Vec<&Hook> = Vec::new();

        // Set internal hook states
        let configs = [
            (&mut self.global_config.hooks, true),
            (&mut self.profile_config.hooks, false),
        ];

        for (hook_list, is_global) in configs {
            hooks.extend(hook_list.iter_mut().map(|(name, h)| {
                h.name = name.to_string();
                h.is_global = is_global;
                &*h
            }));
        }

        self.hook_executor.run_hooks(hooks, output).await
    }

    /// Add a trusted command to the configuration.
    ///
    /// # Arguments
    /// * `trusted_command` - The trusted command to add
    /// * `global` - If true, add to global configuration; otherwise, add to current profile
    ///   configuration
    ///
    /// # Returns
    /// A Result indicating success or an error
    pub async fn add_trusted_command(
        &mut self,
        ctx: &Context,
        trusted_command: TrustedCommand,
        global: bool,
    ) -> Result<()> {
        // Validate the trusted command before adding
        self.validate_trusted_command(&trusted_command)?;
        
        let config = self.get_config_mut(global);
        
        // Initialize trusted_commands if it doesn't exist
        if config.trusted_commands.is_none() {
            config.trusted_commands = Some(TrustedCommandsConfig::default());
        }
        
        // Check for existing command and update description if it exists
        if let Some(ref mut trusted_commands_config) = config.trusted_commands {
            if let Some(existing_cmd) = trusted_commands_config.trusted_commands.iter_mut().find(|cmd| cmd.command == trusted_command.command) {
                // Update the description of the existing command
                existing_cmd.description = trusted_command.description.clone();
                
                // Save the updated configuration
                self.save_config(ctx, global).await
                    .map_err(|e| eyre!("Failed to update trusted command '{}': {}", trusted_command.command, e))?;
                
                tracing::info!("Updated description for trusted command pattern '{}' in {} configuration", 
                              trusted_command.command, if global { "global" } else { "profile" });
                return Ok(());
            }
        }
        
        // Add the new trusted command if it doesn't exist
        config.trusted_commands.as_mut().unwrap().trusted_commands.push(trusted_command.clone());
        
        // Save the updated configuration with error handling
        self.save_config(ctx, global).await
            .map_err(|e| eyre!("Failed to save trusted command '{}': {}", trusted_command.command, e))?;
        
        tracing::info!("Added new trusted command pattern '{}' to {} configuration", 
                      trusted_command.command, if global { "global" } else { "profile" });
        Ok(())
    }
    
    /// Validate a trusted command before adding it to the configuration.
    ///
    /// # Arguments
    /// * `trusted_command` - The trusted command to validate
    ///
    /// # Returns
    /// A Result indicating if the command is valid
    fn validate_trusted_command(&self, trusted_command: &TrustedCommand) -> Result<()> {
        // Check for empty command patterns
        if trusted_command.command.trim().is_empty() {
            return Err(eyre!("Command pattern cannot be empty"));
        }
        
        // Check for dangerous patterns that should never be trusted
        if let Some(pattern_match) = dangerous_patterns::check_all_dangerous_patterns(&trusted_command.command) {
            let reason = match pattern_match.pattern_type {
                dangerous_patterns::DangerousPatternType::Destructive => "destructive command",
                dangerous_patterns::DangerousPatternType::ShellControl => "shell control pattern",
                dangerous_patterns::DangerousPatternType::IoRedirection => "I/O redirection pattern",
            };
            return Err(eyre!("Command pattern '{}' contains dangerous pattern '{}' ({}) and cannot be trusted", 
                            trusted_command.command, pattern_match.pattern, reason));
        }
        
        // Check for invalid regex patterns when converting glob to regex
        let regex_pattern = trusted_command.command
            .replace("*", ".*")
            .replace("?", ".");
        let regex_pattern = format!("^{}$", regex_pattern);
        
        if regex::Regex::new(&regex_pattern).is_err() {
            return Err(eyre!("Command pattern '{}' contains invalid regex syntax", trusted_command.command));
        }
        
        Ok(())
    }
    
    /// Get the trusted commands configuration.
    ///
    /// # Arguments
    /// * `global` - If true, get global configuration; otherwise, get current profile configuration
    ///
    /// # Returns
    /// A TrustedCommandsConfig (owned value)
    pub fn get_trusted_commands(&self, global: bool) -> TrustedCommandsConfig {
        let config = if global {
            &self.global_config
        } else {
            &self.profile_config
        };
        
        config.trusted_commands.as_ref().cloned().unwrap_or_default()
    }
    
    /// Get combined trusted commands from both global and profile configurations.
    ///
    /// # Returns
    /// A TrustedCommandsConfig containing commands from both global and profile configs
    pub fn get_combined_trusted_commands(&self) -> TrustedCommandsConfig {
        let mut combined = TrustedCommandsConfig::default();
        
        // Add global trusted commands first
        if let Some(ref global_trusted) = self.global_config.trusted_commands {
            combined.trusted_commands.extend(global_trusted.trusted_commands.clone());
        }
        
        // Add profile-specific trusted commands
        if let Some(ref profile_trusted) = self.profile_config.trusted_commands {
            // Only add commands that don't already exist (avoid duplicates)
            for cmd in &profile_trusted.trusted_commands {
                if !combined.trusted_commands.iter().any(|existing| existing.command == cmd.command) {
                    combined.trusted_commands.push(cmd.clone());
                }
            }
        }
        
        combined
    }
    
    /// Get processed trusted commands for efficient pattern matching.
    ///
    /// # Returns
    /// A ProcessedTrustedCommands containing combined commands from both global and profile configs
    pub fn get_processed_trusted_commands(&self) -> ProcessedTrustedCommands {
        let combined_config = self.get_combined_trusted_commands();
        ProcessedTrustedCommands::new(combined_config)
    }
    
    /// Remove a trusted command from the configuration.
    ///
    /// # Arguments
    /// * `command_pattern` - The command pattern to remove
    /// * `global` - If true, remove from global configuration; otherwise, remove from current profile
    ///   configuration
    ///
    /// # Returns
    /// A Result indicating success or an error
    pub async fn remove_trusted_command(
        &mut self,
        ctx: &Context,
        command_pattern: &str,
        global: bool,
    ) -> Result<()> {
        let config = self.get_config_mut(global);
        
        if let Some(ref mut trusted_commands_config) = config.trusted_commands {
            let original_len = trusted_commands_config.trusted_commands.len();
            trusted_commands_config.trusted_commands.retain(|cmd| cmd.command != command_pattern);
            
            if trusted_commands_config.trusted_commands.len() < original_len {
                // Save the updated configuration
                self.save_config(ctx, global).await?;
                Ok(())
            } else {
                Err(eyre!("Trusted command pattern '{}' not found", command_pattern))
            }
        } else {
            Err(eyre!("No trusted commands configuration found"))
        }
    }
    
    /// Clear all trusted commands from the configuration.
    ///
    /// # Arguments
    /// * `global` - If true, clear global configuration; otherwise, clear current profile
    ///   configuration
    ///
    /// # Returns
    /// A Result indicating success or an error
    #[allow(dead_code)]
    pub async fn clear_trusted_commands(&mut self, ctx: &Context, global: bool) -> Result<()> {
        let config = self.get_config_mut(global);
        
        if let Some(ref mut trusted_commands_config) = config.trusted_commands {
            trusted_commands_config.trusted_commands.clear();
        } else {
            config.trusted_commands = Some(TrustedCommandsConfig::default());
        }
        
        // Save the updated configuration
        self.save_config(ctx, global).await?;
        
        Ok(())
    }
}

fn profile_dir_path(ctx: &Context, profile_name: &str) -> Result<PathBuf> {
    Ok(directories::chat_profiles_dir(ctx)?.join(profile_name))
}

/// Path to the context config file for `profile_name`.
pub fn profile_context_path(ctx: &Context, profile_name: &str) -> Result<PathBuf> {
    Ok(directories::chat_profiles_dir(ctx)?
        .join(profile_name)
        .join("context.json"))
}

/// Load the global context configuration.
///
/// If the global configuration file doesn't exist, returns a default configuration.
/// Handles errors gracefully by falling back to default configuration when JSON is malformed.
async fn load_global_config(ctx: &Context) -> Result<ContextConfig> {
    let global_path = directories::chat_global_context_path(ctx)?;
    debug!(?global_path, "loading global config");
    
    if ctx.fs.exists(&global_path) {
        match load_config_with_error_handling(ctx, &global_path, "global").await {
            Ok(config) => Ok(config),
            Err(e) => {
                tracing::warn!("Failed to load global configuration, using default: {}", e);
                Ok(get_default_global_config())
            }
        }
    } else {
        Ok(get_default_global_config())
    }
}

/// Get the default global configuration.
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

/// Load configuration from a file with comprehensive error handling.
/// 
/// This function handles various error scenarios gracefully:
/// - File read errors (permissions, I/O issues)
/// - Invalid JSON format
/// - Malformed trusted commands configuration
/// - Missing or invalid fields
async fn load_config_with_error_handling(
    ctx: &Context,
    config_path: &Path,
    config_type: &str,
) -> Result<ContextConfig> {
    // Handle file read errors
    let contents = ctx.fs.read_to_string(config_path).await
        .map_err(|e| eyre!("Failed to read {} configuration file '{}': {}", config_type, config_path.display(), e))?;
    
    // Handle empty files
    if contents.trim().is_empty() {
        tracing::warn!("{} configuration file '{}' is empty, using default configuration", config_type, config_path.display());
        return Ok(ContextConfig::default());
    }
    
    // Parse JSON with detailed error handling
    let mut config: ContextConfig = serde_json::from_str(&contents)
        .map_err(|e| {
            let line_col = format!(" at line {}", e.line());
            eyre!("Invalid JSON format in {} configuration file '{}'{}: {}", 
                  config_type, config_path.display(), line_col, e)
        })?;
    
    // Validate and sanitize trusted commands configuration
    if let Some(ref mut trusted_commands_config) = config.trusted_commands {
        validate_and_sanitize_trusted_commands(trusted_commands_config, config_type, config_path)?;
    }
    
    Ok(config)
}

/// Validate and sanitize trusted commands configuration.
/// 
/// This function:
/// - Validates command patterns for basic safety
/// - Removes invalid or dangerous patterns
/// - Logs warnings for any issues found
fn validate_and_sanitize_trusted_commands(
    trusted_commands_config: &mut TrustedCommandsConfig,
    config_type: &str,
    config_path: &Path,
) -> Result<()> {
    let original_count = trusted_commands_config.trusted_commands.len();
    let mut removed_count = 0;
    
    // Filter out invalid or potentially dangerous patterns
    trusted_commands_config.trusted_commands.retain(|cmd| {
        // Check for empty command patterns
        if cmd.command.trim().is_empty() {
            tracing::warn!("Removing empty command pattern from {} configuration '{}'", 
                          config_type, config_path.display());
            removed_count += 1;
            return false;
        }
        
        // Check for dangerous patterns that should never be trusted
        if let Some(pattern_match) = dangerous_patterns::check_all_dangerous_patterns(&cmd.command) {
            let reason = match pattern_match.pattern_type {
                dangerous_patterns::DangerousPatternType::Destructive => "destructive command",
                dangerous_patterns::DangerousPatternType::ShellControl => "shell control pattern",
                dangerous_patterns::DangerousPatternType::IoRedirection => "I/O redirection pattern",
            };
            tracing::warn!("Removing potentially dangerous command pattern '{}' (contains '{}' - {}) from {} configuration '{}'", 
                          cmd.command, pattern_match.pattern, reason, config_type, config_path.display());
            removed_count += 1;
            return false;
        }
        
        // Check for invalid regex patterns when converting glob to regex
        let regex_pattern = cmd.command
            .replace("*", ".*")
            .replace("?", ".");
        let regex_pattern = format!("^{}$", regex_pattern);
        
        if regex::Regex::new(&regex_pattern).is_err() {
            tracing::warn!("Removing command pattern '{}' with invalid regex from {} configuration '{}'", 
                          cmd.command, config_type, config_path.display());
            removed_count += 1;
            return false;
        }
        
        true
    });
    
    if removed_count > 0 {
        tracing::warn!("Removed {} invalid/dangerous trusted command patterns from {} configuration '{}' (originally had {})", 
                      removed_count, config_type, config_path.display(), original_count);
    }
    
    Ok(())
}

/// Load a profile's context configuration.
///
/// If the profile configuration file doesn't exist, creates a default configuration.
/// Handles errors gracefully by falling back to default configuration when JSON is malformed.
async fn load_profile_config(ctx: &Context, profile_name: &str) -> Result<ContextConfig> {
    let profile_path = profile_context_path(ctx, profile_name)?;
    debug!(?profile_path, "loading profile config");
    
    if ctx.fs.exists(&profile_path) {
        match load_config_with_error_handling(ctx, &profile_path, &format!("profile '{}'", profile_name)).await {
            Ok(config) => Ok(config),
            Err(e) => {
                tracing::warn!("Failed to load profile '{}' configuration, using default: {}", profile_name, e);
                Ok(ContextConfig::default())
            }
        }
    } else {
        // Return empty configuration for new profiles
        Ok(ContextConfig::default())
    }
}

/// Process a path, handling glob patterns and file types.
///
/// This method:
/// 1. Expands the path (handling ~ for home directory)
/// 2. If the path contains glob patterns, expands them
/// 3. For each resulting path, adds the file to the context collection
/// 4. Handles directories by including all files in the directory (non-recursive)
/// 5. With force=true, includes paths that don't exist yet
///
/// # Arguments
/// * `path` - The path to process
/// * `context_files` - The collection to add files to
/// * `is_validation` - If true, error when glob patterns don't match; if false, silently skip
///
/// # Returns
/// A Result indicating success or an error
async fn process_path(
    ctx: &Context,
    path: &str,
    context_files: &mut Vec<(String, String)>,
    is_validation: bool,
) -> Result<()> {
    // Expand ~ to home directory
    let expanded_path = if path.starts_with('~') {
        if let Some(home_dir) = ctx.env.home() {
            home_dir.join(&path[2..]).to_string_lossy().to_string()
        } else {
            return Err(eyre!("Could not determine home directory"));
        }
    } else {
        path.to_string()
    };

    // Handle absolute, relative paths, and glob patterns
    let full_path = if expanded_path.starts_with('/') {
        expanded_path
    } else {
        ctx.env
            .current_dir()?
            .join(&expanded_path)
            .to_string_lossy()
            .to_string()
    };

    // Required in chroot testing scenarios so that we can use `Path::exists`.
    let full_path = ctx.fs.chroot_path_str(full_path);

    // Check if the path contains glob patterns
    if full_path.contains('*') || full_path.contains('?') || full_path.contains('[') {
        // Expand glob pattern
        match glob(&full_path) {
            Ok(entries) => {
                let mut found_any = false;

                for entry in entries {
                    match entry {
                        Ok(path) => {
                            if path.is_file() {
                                add_file_to_context(ctx, &path, context_files).await?;
                                found_any = true;
                            }
                        },
                        Err(e) => return Err(eyre!("Glob error: {}", e)),
                    }
                }

                if !found_any && is_validation {
                    // When validating paths (e.g., for /context add), error if no files match
                    return Err(eyre!("No files found matching glob pattern '{}'", full_path));
                }
                // When just showing expanded files (e.g., for /context show --expand),
                // silently skip non-matching patterns (don't add anything to context_files)
            },
            Err(e) => return Err(eyre!("Invalid glob pattern '{}': {}", full_path, e)),
        }
    } else {
        // Regular path
        let path = Path::new(&full_path);
        if path.exists() {
            if path.is_file() {
                add_file_to_context(ctx, path, context_files).await?;
            } else if path.is_dir() {
                // For directories, add all files in the directory (non-recursive)
                let mut read_dir = ctx.fs.read_dir(path).await?;
                while let Some(entry) = read_dir.next_entry().await? {
                    let path = entry.path();
                    if path.is_file() {
                        add_file_to_context(ctx, &path, context_files).await?;
                    }
                }
            }
        } else if is_validation {
            // When validating paths (e.g., for /context add), error if the path doesn't exist
            return Err(eyre!("Path '{}' does not exist", full_path));
        }
    }

    Ok(())
}

/// Add a file to the context collection.
///
/// This method:
/// 1. Reads the content of the file
/// 2. Adds the (filename, content) pair to the context collection
///
/// # Arguments
/// * `path` - The path to the file
/// * `context_files` - The collection to add the file to
///
/// # Returns
/// A Result indicating success or an error
async fn add_file_to_context(ctx: &Context, path: &Path, context_files: &mut Vec<(String, String)>) -> Result<()> {
    let filename = path.to_string_lossy().to_string();
    let content = ctx.fs.read_to_string(path).await?;
    context_files.push((filename, content));
    Ok(())
}

/// Validate a profile name.
///
/// Profile names can only contain alphanumeric characters, hyphens, and underscores.
///
/// # Arguments
/// * `name` - Name to validate
///
/// # Returns
/// A Result indicating if the name is valid
fn validate_profile_name(name: &str) -> Result<()> {
    // Check if name is empty
    if name.is_empty() {
        return Err(eyre!("Profile name cannot be empty"));
    }

    // Check if name contains only allowed characters and starts with an alphanumeric character
    let re = Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9_-]*$").unwrap();
    if !re.is_match(name) {
        return Err(eyre!(
            "Profile name must start with an alphanumeric character and can only contain alphanumeric characters, hyphens, and underscores"
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::chat::util::test::create_test_context_manager;

    #[tokio::test]
    async fn test_validate_profile_name() {
        // Test valid names
        assert!(validate_profile_name("valid").is_ok());
        assert!(validate_profile_name("valid-name").is_ok());
        assert!(validate_profile_name("valid_name").is_ok());
        assert!(validate_profile_name("valid123").is_ok());
        assert!(validate_profile_name("1valid").is_ok());
        assert!(validate_profile_name("9test").is_ok());

        // Test invalid names
        assert!(validate_profile_name("").is_err());
        assert!(validate_profile_name("invalid/name").is_err());
        assert!(validate_profile_name("invalid.name").is_err());
        assert!(validate_profile_name("invalid name").is_err());
        assert!(validate_profile_name("_invalid").is_err());
        assert!(validate_profile_name("-invalid").is_err());
    }

    #[tokio::test]
    async fn test_profile_ops() -> Result<()> {
        let ctx = Context::new();
        let mut manager = create_test_context_manager(None).await?;

        assert_eq!(manager.current_profile, "default");

        // Create ops
        manager.create_profile(&ctx, "test_profile").await?;
        assert!(profile_context_path(&ctx, "test_profile")?.exists());
        assert!(manager.create_profile(&ctx, "test_profile").await.is_err());
        manager.create_profile(&ctx, "alt").await?;

        // Listing
        let profiles = manager.list_profiles(&ctx).await?;
        assert!(profiles.contains(&"default".to_string()));
        assert!(profiles.contains(&"test_profile".to_string()));
        assert!(profiles.contains(&"alt".to_string()));

        // Switching
        manager.switch_profile(&ctx, "test_profile").await?;
        assert!(manager.switch_profile(&ctx, "notexists").await.is_err());

        // Renaming
        manager.rename_profile(&ctx, "alt", "renamed").await?;
        assert!(!profile_context_path(&ctx, "alt")?.exists());
        assert!(profile_context_path(&ctx, "renamed")?.exists());

        // Delete ops
        assert!(manager.delete_profile(&ctx, "test_profile").await.is_err());
        manager.switch_profile(&ctx, "default").await?;
        manager.delete_profile(&ctx, "test_profile").await?;
        assert!(!profile_context_path(&ctx, "test_profile")?.exists());
        assert!(manager.delete_profile(&ctx, "test_profile").await.is_err());
        assert!(manager.delete_profile(&ctx, "default").await.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_collect_exceeds_limit() -> Result<()> {
        let ctx = Context::new();
        let mut manager = create_test_context_manager(Some(2)).await?;

        ctx.fs.create_dir_all("test").await?;
        ctx.fs.write("test/to-include.md", "ha").await?;
        ctx.fs
            .write("test/to-drop.md", "long content that exceed limit")
            .await?;
        manager
            .add_paths(&ctx, vec!["test/*.md".to_string()], false, false)
            .await?;

        let (used, dropped) = manager.collect_context_files_with_limit(&ctx).await.unwrap();

        assert!(used.len() + dropped.len() == 2);
        assert!(used.len() == 1);
        assert!(dropped.len() == 1);
        Ok(())
    }

    #[tokio::test]
    async fn test_path_ops() -> Result<()> {
        let ctx = Context::new();
        let mut manager = create_test_context_manager(None).await?;

        // Create some test files for matching.
        ctx.fs.create_dir_all("test").await?;
        ctx.fs.write("test/p1.md", "p1").await?;
        ctx.fs.write("test/p2.md", "p2").await?;

        assert!(
            manager.get_context_files(&ctx).await?.is_empty(),
            "no files should be returned for an empty profile when force is false"
        );

        manager
            .add_paths(&ctx, vec!["test/*.md".to_string()], false, false)
            .await?;
        let files = manager.get_context_files(&ctx).await?;
        assert!(files[0].0.ends_with("p1.md"));
        assert_eq!(files[0].1, "p1");
        assert!(files[1].0.ends_with("p2.md"));
        assert_eq!(files[1].1, "p2");

        assert!(
            manager
                .add_paths(&ctx, vec!["test/*.txt".to_string()], false, false)
                .await
                .is_err(),
            "adding a glob with no matching and without force should fail"
        );

        Ok(())
    }

    #[test]
    fn test_trusted_command_deserialization() {
        // Test basic trusted command deserialization
        let json = r#"{
            "command": "npm *",
            "description": "All npm commands"
        }"#;
        
        let cmd: TrustedCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.command, "npm *");
        assert_eq!(cmd.description, Some("All npm commands".to_string()));
        
        // Test without description
        let json = r#"{
            "command": "git status"
        }"#;
        
        let cmd: TrustedCommand = serde_json::from_str(json).unwrap();
        assert_eq!(cmd.command, "git status");
        assert_eq!(cmd.description, None);
    }
    
    #[test]
    fn test_trusted_commands_config_deserialization() {
        let json = r#"{
            "trusted_commands": [
                {
                    "command": "npm *",
                    "description": "All npm commands"
                },
                {
                    "command": "git status"
                }
            ]
        }"#;
        
        let config: TrustedCommandsConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.trusted_commands.len(), 2);
        assert_eq!(config.trusted_commands[0].command, "npm *");
        assert_eq!(config.trusted_commands[0].description, Some("All npm commands".to_string()));
        assert_eq!(config.trusted_commands[1].command, "git status");
        assert_eq!(config.trusted_commands[1].description, None);
    }
    
    #[test]
    fn test_context_config_with_trusted_commands() {
        let json = r#"{
            "paths": ["README.md"],
            "hooks": {},
            "trusted_commands": {
                "trusted_commands": [
                    {
                        "command": "npm *",
                        "description": "All npm commands"
                    }
                ]
            }
        }"#;
        
        let config: ContextConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.paths, vec!["README.md"]);
        assert!(config.trusted_commands.is_some());
        
        let trusted_commands = config.trusted_commands.unwrap();
        assert_eq!(trusted_commands.trusted_commands.len(), 1);
        assert_eq!(trusted_commands.trusted_commands[0].command, "npm *");
    }
    
    #[test]
    fn test_processed_trusted_commands_exact_match() {
        let config = TrustedCommandsConfig {
            trusted_commands: vec![
                TrustedCommand {
                    command: "git status".to_string(),
                    description: Some("Git status command".to_string()),
                },
                TrustedCommand {
                    command: "npm install".to_string(),
                    description: None,
                },
            ],
        };
        
        let processed = ProcessedTrustedCommands::new(config);
        
        // Test exact matches
        assert!(processed.is_trusted("git status"));
        assert!(processed.is_trusted("npm install"));
        
        // Test non-matches
        assert!(!processed.is_trusted("git commit"));
        assert!(!processed.is_trusted("npm run"));
        assert!(!processed.is_trusted("ls"));
    }
    
    #[test]
    fn test_processed_trusted_commands_glob_match() {
        let config = TrustedCommandsConfig {
            trusted_commands: vec![
                TrustedCommand {
                    command: "npm *".to_string(),
                    description: Some("All npm commands".to_string()),
                },
                TrustedCommand {
                    command: "git restore *".to_string(),
                    description: Some("All git restore commands".to_string()),
                },
            ],
        };
        
        let processed = ProcessedTrustedCommands::new(config);
        
        // Test glob matches
        assert!(processed.is_trusted("npm install"));
        assert!(processed.is_trusted("npm run build"));
        assert!(processed.is_trusted("npm install --save-dev typescript"));
        assert!(processed.is_trusted("git restore file.txt"));
        assert!(processed.is_trusted("git restore --staged file.txt"));
        
        // Test non-matches
        assert!(!processed.is_trusted("git status"));
        assert!(!processed.is_trusted("git commit"));
        assert!(!processed.is_trusted("yarn install"));
        assert!(!processed.is_trusted("ls"));
    }
    
    #[test]
    fn test_processed_trusted_commands_mixed_patterns() {
        let config = TrustedCommandsConfig {
            trusted_commands: vec![
                TrustedCommand {
                    command: "git status".to_string(),
                    description: None,
                },
                TrustedCommand {
                    command: "git restore *".to_string(),
                    description: None,
                },
                TrustedCommand {
                    command: "npm *".to_string(),
                    description: None,
                },
            ],
        };
        
        let processed = ProcessedTrustedCommands::new(config);
        
        // Test exact match
        assert!(processed.is_trusted("git status"));
        
        // Test glob matches
        assert!(processed.is_trusted("git restore file.txt"));
        assert!(processed.is_trusted("npm install"));
        
        // Test non-matches
        assert!(!processed.is_trusted("git commit"));
        assert!(!processed.is_trusted("yarn install"));
    }
    
    #[test]
    fn test_glob_match_edge_cases() {
        let config = TrustedCommandsConfig {
            trusted_commands: vec![
                TrustedCommand {
                    command: "*".to_string(),
                    description: None,
                },
            ],
        };
        
        let processed = ProcessedTrustedCommands::new(config);
        
        // Test that * matches everything
        assert!(processed.is_trusted("any command"));
        assert!(processed.is_trusted("git status"));
        assert!(processed.is_trusted("npm install"));
        
        // Test empty pattern
        let config = TrustedCommandsConfig {
            trusted_commands: vec![
                TrustedCommand {
                    command: "".to_string(),
                    description: None,
                },
            ],
        };
        
        let processed = ProcessedTrustedCommands::new(config);
        assert!(processed.is_trusted(""));
        assert!(!processed.is_trusted("git status"));
    }
    
    #[tokio::test]
    async fn test_trusted_commands_management() -> Result<()> {
        let ctx = Context::new();
        let mut manager = create_test_context_manager(None).await?;
        
        // Test adding trusted commands
        let cmd1 = TrustedCommand {
            command: "npm *".to_string(),
            description: Some("All npm commands".to_string()),
        };
        
        let cmd2 = TrustedCommand {
            command: "git status".to_string(),
            description: None,
        };
        
        // Add to profile config
        manager.add_trusted_command(&ctx, cmd1.clone(), false).await?;
        manager.add_trusted_command(&ctx, cmd2.clone(), false).await?;
        
        // Test getting trusted commands
        let profile_commands = manager.get_trusted_commands(false);
        assert_eq!(profile_commands.trusted_commands.len(), 2);
        assert_eq!(profile_commands.trusted_commands[0].command, "npm *");
        assert_eq!(profile_commands.trusted_commands[1].command, "git status");
        
        // Test updating existing command description
        let updated_cmd1 = TrustedCommand {
            command: "npm *".to_string(),
            description: Some("Updated npm commands description".to_string()),
        };
        let update_result = manager.add_trusted_command(&ctx, updated_cmd1, false).await;
        assert!(update_result.is_ok());
        
        // Verify the description was updated
        let profile_commands = manager.get_trusted_commands(false);
        assert_eq!(profile_commands.trusted_commands.len(), 2); // Still 2 commands
        let npm_cmd = profile_commands.trusted_commands.iter().find(|cmd| cmd.command == "npm *").unwrap();
        assert_eq!(npm_cmd.description, Some("Updated npm commands description".to_string()));
        
        // Test adding to global config
        let global_cmd = TrustedCommand {
            command: "ls *".to_string(),
            description: Some("All ls commands".to_string()),
        };
        manager.add_trusted_command(&ctx, global_cmd, true).await?;
        
        // Test combined commands
        let combined = manager.get_combined_trusted_commands();
        assert_eq!(combined.trusted_commands.len(), 3);
        
        // Test removing commands
        manager.remove_trusted_command(&ctx, "git status", false).await?;
        let profile_commands = manager.get_trusted_commands(false);
        assert_eq!(profile_commands.trusted_commands.len(), 1);
        assert_eq!(profile_commands.trusted_commands[0].command, "npm *");
        
        // Test clearing commands
        manager.clear_trusted_commands(&ctx, false).await?;
        let profile_commands = manager.get_trusted_commands(false);
        assert_eq!(profile_commands.trusted_commands.len(), 0);
        
        Ok(())
    }
    
    #[tokio::test]
    async fn test_trusted_commands_error_handling() -> Result<()> {
        let ctx = Context::new();
        let mut manager = create_test_context_manager(None).await?;
        
        // Test validation of empty command patterns
        let empty_cmd = TrustedCommand {
            command: "".to_string(),
            description: None,
        };
        let result = manager.add_trusted_command(&ctx, empty_cmd, false).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot be empty"));
        
        // Test validation of dangerous patterns
        let dangerous_cmd = TrustedCommand {
            command: "rm -rf /".to_string(),
            description: None,
        };
        let result = manager.add_trusted_command(&ctx, dangerous_cmd, false).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("dangerous pattern"));
        
        // Test validation of invalid regex patterns
        let invalid_regex_cmd = TrustedCommand {
            command: "test[".to_string(), // Invalid regex due to unclosed bracket
            description: None,
        };
        let result = manager.add_trusted_command(&ctx, invalid_regex_cmd, false).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid regex"));
        
        Ok(())
    }
    
    #[tokio::test]
    async fn test_config_loading_error_handling() -> Result<()> {
        let ctx = Context::new();
        let temp_dir = ctx.fs.create_tempdir().await?;
        
        // Test loading invalid JSON
        let invalid_json_path = temp_dir.path().join("invalid.json");
        ctx.fs.write(&invalid_json_path, "{ invalid json }").await?;
        
        let result = load_config_with_error_handling(&ctx, &invalid_json_path, "test").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid JSON format"));
        
        // Test loading empty file
        let empty_file_path = temp_dir.path().join("empty.json");
        ctx.fs.write(&empty_file_path, "").await?;
        
        let result = load_config_with_error_handling(&ctx, &empty_file_path, "test").await;
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.paths.len(), 0); // Should be default empty config
        
        // Test loading file with dangerous trusted commands
        let dangerous_config = r#"{
            "paths": [],
            "hooks": {},
            "trusted_commands": {
                "trusted_commands": [
                    {
                        "command": "rm -rf /",
                        "description": "Dangerous command"
                    },
                    {
                        "command": "ls -la",
                        "description": "Safe command"
                    }
                ]
            }
        }"#;
        
        let dangerous_file_path = temp_dir.path().join("dangerous.json");
        ctx.fs.write(&dangerous_file_path, dangerous_config).await?;
        
        let result = load_config_with_error_handling(&ctx, &dangerous_file_path, "test").await;
        assert!(result.is_ok());
        let config = result.unwrap();
        
        // Should have removed the dangerous command but kept the safe one
        let trusted_commands = config.trusted_commands.unwrap();
        assert_eq!(trusted_commands.trusted_commands.len(), 1);
        assert_eq!(trusted_commands.trusted_commands[0].command, "ls -la");
        
        Ok(())
    }
    
    #[test]
    fn test_validate_and_sanitize_trusted_commands() {
        use std::path::Path;
        
        let mut config = TrustedCommandsConfig {
            trusted_commands: vec![
                TrustedCommand {
                    command: "".to_string(), // Empty - should be removed
                    description: None,
                },
                TrustedCommand {
                    command: "rm -rf /".to_string(), // Dangerous - should be removed
                    description: None,
                },
                TrustedCommand {
                    command: "test[".to_string(), // Invalid regex - should be removed
                    description: None,
                },
                TrustedCommand {
                    command: "ls -la".to_string(), // Safe - should be kept
                    description: None,
                },
                TrustedCommand {
                    command: "npm *".to_string(), // Safe with glob - should be kept
                    description: None,
                },
            ],
        };
        
        let result = validate_and_sanitize_trusted_commands(&mut config, "test", Path::new("/test/path"));
        assert!(result.is_ok());
        
        // Should have kept only the 2 safe commands
        assert_eq!(config.trusted_commands.len(), 2);
        assert_eq!(config.trusted_commands[0].command, "ls -la");
        assert_eq!(config.trusted_commands[1].command, "npm *");
    }
}
