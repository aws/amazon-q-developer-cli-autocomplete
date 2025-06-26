#![allow(dead_code)]

use std::borrow::Borrow;
use std::collections::{
    HashMap,
    HashSet,
};
use std::ffi::OsStr;
use std::io::{
    self,
    Write,
};
use std::path::{
    Path,
    PathBuf,
};

use crossterm::style::Stylize as _;
use crossterm::{
    execute,
    queue,
    style,
};
use eyre::bail;
use regex::Regex;
use serde::{
    Deserialize,
    Serialize,
};
use tokio::fs::ReadDir;
use tracing::error;

use super::chat::tools::custom_tool::CustomToolConfig;
use super::chat::tools::{
    DEFAULT_APPROVE,
    NATIVE_TOOLS,
    ToolOrigin,
};
use crate::cli::chat::cli::hooks::{
    Hook,
    HookTrigger,
};
use crate::cli::chat::context::ContextConfig;
use crate::os::Os;
use crate::util::{
    MCP_SERVER_TOOL_DELIMITER,
    directories,
};

// This is to mirror claude's config set up
#[derive(Clone, Serialize, Deserialize, Debug, Default, Eq, PartialEq)]
#[serde(rename_all = "camelCase", transparent)]
pub struct McpServerConfig {
    pub mcp_servers: HashMap<String, CustomToolConfig>,
}

impl McpServerConfig {
    pub async fn load_from_file(os: &Os, path: impl AsRef<Path>) -> eyre::Result<Self> {
        let contents = os.fs.read_to_string(path.as_ref()).await?;
        Ok(serde_json::from_str(&contents)?)
    }

    pub async fn save_to_file(&self, os: &Os, path: impl AsRef<Path>) -> eyre::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        os.fs.write(path.as_ref(), json).await?;
        Ok(())
    }

    #[allow(dead_code)]
    fn from_slice(slice: &[u8], output: &mut impl Write, location: &str) -> eyre::Result<McpServerConfig> {
        match serde_json::from_slice::<Self>(slice) {
            Ok(config) => Ok(config),
            Err(e) => {
                queue!(
                    output,
                    style::SetForegroundColor(style::Color::Yellow),
                    style::Print("WARNING: "),
                    style::ResetColor,
                    style::Print(format!("Error reading {location} mcp config: {e}\n")),
                    style::Print("Please check to make sure config is correct. Discarding.\n"),
                )?;
                Ok(McpServerConfig::default())
            },
        }
    }
}

/// An [Agent] is a declarative way of configuring a given instance of q chat. Currently, it is
/// impacting q chat in via influenicng [ContextManager] and [ToolManager].
/// Changes made to [ContextManager] and [ToolManager] do not persist across sessions.
#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    /// Agent or persona names are derived from the file name. Thus they are skipped for
    /// serializing
    #[serde(skip)]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub mcp_servers: McpServerConfig,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub alias: HashMap<String, String>,
    #[serde(default)]
    pub allowed_tools: HashSet<String>,
    #[serde(default)]
    pub included_files: Vec<String>,
    #[serde(default)]
    pub create_hooks: serde_json::Value,
    #[serde(default)]
    pub prompt_hooks: serde_json::Value,
    #[serde(default)]
    pub tools_settings: HashMap<String, serde_json::Value>,
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

impl Default for Agent {
    fn default() -> Self {
        Self {
            name: "default".to_string(),
            description: Some("Default agent".to_string()),
            prompt: Default::default(),
            mcp_servers: Default::default(),
            tools: NATIVE_TOOLS.iter().copied().map(str::to_string).collect::<Vec<_>>(),
            alias: Default::default(),
            allowed_tools: {
                let mut set = HashSet::<String>::new();
                let default_approve = DEFAULT_APPROVE.iter().copied().map(str::to_string);
                set.extend(default_approve);
                set
            },
            included_files: vec!["AmazonQ.md", "README.md", ".amazonq/rules/**/*.md"]
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>(),
            create_hooks: Default::default(),
            prompt_hooks: Default::default(),
            tools_settings: Default::default(),
            path: None,
        }
    }
}

impl Agent {
    /// Retrieves an agent by name. It does so via first seeking the given agent under local dir,
    /// and falling back to global dir if it does not exist in local.
    pub async fn get_agent_by_name(os: &Os, agent_name: &str) -> eyre::Result<(Agent, PathBuf)> {
        let config_path: Result<PathBuf, PathBuf> = 'config: {
            // local first, and then fall back to looking at global
            let local_config_dir = directories::chat_local_persona_dir()?.join(agent_name);
            if os.fs.exists(&local_config_dir) {
                break 'config Ok::<PathBuf, PathBuf>(local_config_dir);
            }

            let global_config_dir = directories::chat_global_persona_path(os)?.join(format!("{agent_name}.json"));
            if os.fs.exists(&global_config_dir) {
                break 'config Ok(global_config_dir);
            }

            Err(global_config_dir)
        };

        match config_path {
            Ok(config_path) => {
                let content = os.fs.read(&config_path).await?;
                Ok((serde_json::from_slice::<Agent>(&content)?, config_path))
            },
            Err(global_config_dir) if agent_name == "default" => {
                os.fs
                    .create_dir_all(
                        global_config_dir
                            .parent()
                            .ok_or(eyre::eyre!("Failed to retrieve global agent config parent path"))?,
                    )
                    .await?;
                os.fs.create_new(&global_config_dir).await?;

                let default_agent = Agent::default();
                let content = serde_json::to_string_pretty(&default_agent)?;
                os.fs.write(&global_config_dir, content.as_bytes()).await?;

                Ok((default_agent, global_config_dir))
            },
            _ => bail!("Agent {agent_name} does not exist"),
        }
    }
}

#[derive(Debug)]
pub enum PermissionEvalResult {
    Allow,
    Ask,
    Deny,
}

#[derive(Clone, Default, Debug)]
pub struct Agents {
    pub agents: HashMap<String, Agent>,
    pub active_idx: String,
    pub trust_all_tools: bool,
}

impl Agents {
    /// This function assumes the relevant transformation to the tool names have been done:
    /// - model tool name -> host tool name
    /// - custom tool namespacing
    pub fn trust_tools(&mut self, tool_names: Vec<String>) {
        if let Some(agent) = self.get_active_mut() {
            agent.allowed_tools.extend(tool_names);
        }
    }

    /// This function assumes the relevant transformation to the tool names have been done:
    /// - model tool name -> host tool name
    /// - custom tool namespacing
    pub fn untrust_tools(&mut self, tool_names: &[String]) {
        if let Some(agent) = self.get_active_mut() {
            agent.allowed_tools.retain(|t| !tool_names.contains(t));
        }
    }

    pub fn get_active(&self) -> Option<&Agent> {
        self.agents.get(&self.active_idx)
    }

    pub fn get_active_mut(&mut self) -> Option<&mut Agent> {
        self.agents.get_mut(&self.active_idx)
    }

    pub fn switch(&mut self, name: &str) -> eyre::Result<&Agent> {
        if !self.agents.contains_key(name) {
            eyre::bail!("No agent with name {name} found");
        }
        self.active_idx = name.to_string();
        self.agents
            .get(name)
            .ok_or(eyre::eyre!("No agent with name {name} found"))
    }

    /// Migrated from [reload_profiles] from context.rs. It loads the active persona from disk and
    /// replaces its in-memory counterpart with it.
    pub async fn reload_personas(&mut self, os: &Os, output: &mut impl Write) -> eyre::Result<()> {
        let persona_name = self.get_active().map(|a| a.name.as_str());
        let mut new_self = Self::load(os, persona_name, output).await;
        std::mem::swap(self, &mut new_self);
        Ok(())
    }

    pub fn list_personas(&self) -> eyre::Result<Vec<String>> {
        Ok(self.agents.keys().cloned().collect::<Vec<_>>())
    }

    /// Migrated from [create_profile] from context.rs, which was creating profiles under the
    /// global directory. We shall preserve this implicit behavior for now until further notice.
    pub async fn create_persona(&mut self, os: &Os, name: &str) -> eyre::Result<()> {
        validate_persona_name(name)?;

        let persona_path = directories::chat_global_persona_path(os)?.join(format!("{name}.json"));
        if persona_path.exists() {
            return Err(eyre::eyre!("Persona '{}' already exists", name));
        }

        let agent = Agent {
            name: name.to_string(),
            path: Some(persona_path.clone()),
            ..Default::default()
        };
        let contents = serde_json::to_string_pretty(&agent)
            .map_err(|e| eyre::eyre!("Failed to serialize profile configuration: {}", e))?;

        if let Some(parent) = persona_path.parent() {
            os.fs.create_dir_all(parent).await?;
        }
        os.fs.write(&persona_path, contents).await?;

        self.agents.insert(name.to_string(), agent);

        Ok(())
    }

    /// Migrated from [delete_profile] from context.rs, which was deleting profiles under the
    /// global directory. We shall preserve this implicit behavior for now until further notice.
    pub async fn delete_persona(&mut self, os: &Os, name: &str) -> eyre::Result<()> {
        if name == self.active_idx.as_str() {
            eyre::bail!("Cannot delete the active persona. Switch to another persona first");
        }

        let to_delete = self
            .agents
            .get(name)
            .ok_or(eyre::eyre!("Persona '{name}' does not exist"))?;
        match to_delete.path.as_ref() {
            Some(path) if path.exists() => {
                os.fs.remove_file(path).await?;
            },
            _ => eyre::bail!("Persona {name} does not have an associated path"),
        }

        self.agents.remove(name);

        Ok(())
    }

    /// Migrated from [load] from context.rs, which was loading profiles under the
    /// local and global directory. We shall preserve this implicit behavior for now until further
    /// notice.
    /// In addition to loading, this function also calls the function responsible for migrating
    /// existing context into agent.
    pub async fn load(os: &Os, agent_name: Option<&str>, output: &mut impl Write) -> Self {
        let mut local_agents = 'local: {
            let Ok(path) = directories::chat_local_persona_dir() else {
                break 'local Vec::<Agent>::new();
            };
            let Ok(files) = tokio::fs::read_dir(path).await else {
                break 'local Vec::<Agent>::new();
            };
            load_agents_from_entries(files).await
        };

        let mut global_agents = 'global: {
            let Ok(path) = directories::chat_global_persona_path(os) else {
                break 'global Vec::<Agent>::new();
            };
            let files = match tokio::fs::read_dir(&path).await {
                Ok(files) => files,
                Err(e) => {
                    if matches!(e.kind(), io::ErrorKind::NotFound) {
                        if let Err(e) = os.fs.create_dir_all(&path).await {
                            error!("Error creating global persona dir: {:?}", e);
                        }
                    }
                    break 'global Vec::<Agent>::new();
                },
            };
            load_agents_from_entries(files).await
        };

        let local_names = local_agents.iter().map(|a| a.name.as_str()).collect::<HashSet<&str>>();
        global_agents.retain(|a| {
            // If there is a naming conflict for agents, we would retain the local instance
            let name = a.name.as_str();
            if local_names.contains(name) {
                let _ = queue!(
                    output,
                    style::SetForegroundColor(style::Color::Yellow),
                    style::Print("WARNING: "),
                    style::ResetColor,
                    style::Print("Persona conflict for "),
                    style::SetForegroundColor(style::Color::Green),
                    style::Print(name),
                    style::ResetColor,
                    style::Print(". Using workspace version.\n")
                );
                false
            } else {
                true
            }
        });

        let _ = output.flush();
        local_agents.append(&mut global_agents);

        // Ensure that we always have a default persona under the global directory
        if !local_agents.iter().any(|a| a.name == "default") {
            let default_agent = Agent {
                path: directories::chat_global_persona_path(os)
                    .ok()
                    .map(|p| p.join("default.json")),
                ..Default::default()
            };

            match serde_json::to_string_pretty(&default_agent) {
                Ok(content) => {
                    if let Ok(path) = directories::chat_global_persona_path(os) {
                        let default_path = path.join("default.json");
                        if let Err(e) = tokio::fs::write(default_path, &content).await {
                            error!("Error writing default persona to file: {:?}", e);
                        }
                    };
                },
                Err(e) => {
                    error!("Error serializing default persona: {:?}", e);
                },
            }

            local_agents.push(default_agent);
        }

        let default_agent = local_agents
            .iter_mut()
            .find(|a| a.name == "default")
            .expect("Missing default agent");

        if let Some(mut migrated_agents) = migrate_context(os, default_agent, output).await {
            local_agents.append(&mut migrated_agents);
        }

        Self {
            agents: local_agents
                .into_iter()
                .map(|a| (a.name.clone(), a))
                .collect::<HashMap<_, _>>(),
            active_idx: agent_name.unwrap_or("default").to_string(),
            ..Default::default()
        }
    }

    /// Returns a label to describe the permission status for a given tool.
    pub fn display_label(&self, tool_name: &str, origin: &ToolOrigin) -> String {
        let tool_trusted = self.get_active().is_some_and(|a| {
            a.allowed_tools.iter().any(|name| {
                // Here the tool names can take the following forms:
                // - @{server_name}{delimiter}{tool_name}
                // - native_tool_name
                name == tool_name
                    || name.strip_prefix("@").is_some_and(|remainder| {
                        remainder
                            .split_once(MCP_SERVER_TOOL_DELIMITER)
                            .is_some_and(|(_left, right)| right == tool_name)
                            || remainder == <ToolOrigin as Borrow<str>>::borrow(origin)
                    })
            })
        });

        if tool_trusted || self.trust_all_tools {
            format!("* {}", "trusted".dark_green().bold())
        } else {
            self.default_permission_label(tool_name)
        }
    }

    /// Provide default permission labels for the built-in set of tools.
    // This "static" way avoids needing to construct a tool instance.
    fn default_permission_label(&self, tool_name: &str) -> String {
        let label = match tool_name {
            "fs_read" => "trusted".dark_green().bold(),
            "fs_write" => "not trusted".dark_grey(),
            #[cfg(not(windows))]
            "execute_bash" => "trust read-only commands".dark_grey(),
            #[cfg(windows)]
            "execute_cmd" => "trust read-only commands".dark_grey(),
            "use_aws" => "trust read-only commands".dark_grey(),
            "report_issue" => "trusted".dark_green().bold(),
            "thinking" => "trusted (prerelease)".dark_green().bold(),
            _ if self.trust_all_tools => "trusted".dark_grey().bold(),
            _ => "not trusted".dark_grey(),
        };

        format!("{} {label}", "*".reset())
    }
}

async fn load_agents_from_entries(mut files: ReadDir) -> Vec<Agent> {
    let mut res = Vec::<Agent>::new();
    while let Ok(Some(file)) = files.next_entry().await {
        let file_path = &file.path();
        if file_path
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|s| s == "json")
        {
            let content = match tokio::fs::read(file_path).await {
                Ok(content) => content,
                Err(e) => {
                    let file_path = file_path.to_string_lossy();
                    tracing::error!("Error reading persona file {file_path}: {:?}", e);
                    continue;
                },
            };
            let mut agent = match serde_json::from_slice::<Agent>(&content) {
                Ok(mut agent) => {
                    agent.path = Some(file_path.clone());
                    agent
                },
                Err(e) => {
                    let file_path = file_path.to_string_lossy();
                    tracing::error!("Error deserializing persona file {file_path}: {:?}", e);
                    continue;
                },
            };
            if let Some(name) = Path::new(&file.file_name()).file_stem() {
                agent.name = name.to_string_lossy().to_string();
                res.push(agent);
            } else {
                let file_path = file_path.to_string_lossy();
                tracing::error!("Unable to determine persona name from config file at {file_path}, skipping");
            }
        }
    }
    res
}

fn validate_persona_name(name: &str) -> eyre::Result<()> {
    // Check if name is empty
    if name.is_empty() {
        eyre::bail!("Persona name cannot be empty");
    }

    // Check if name contains only allowed characters and starts with an alphanumeric character
    let re = Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9_-]*$")?;
    if !re.is_match(name) {
        eyre::bail!(
            "Persona name must start with an alphanumeric character and can only contain alphanumeric characters, hyphens, and underscores"
        );
    }

    Ok(())
}

/// Migration of context consists of the following:
/// 1. Scan for global context config. If it exists, move it into default
/// 2. If global context config exists, move it to a backup
/// 3. Scan for workspace context config. Create an agent for each config found respectively. Each
///    config created shall have its context combined with the aforementioned global context.
/// 4. Move all workspace context config found to a backup.
/// 5. Return all new agents created from the migration.
async fn migrate_context(os: &Os, default_agent: &mut Agent, output: &mut impl Write) -> Option<Vec<Agent>> {
    let legacy_global_config_path = directories::chat_global_context_path(os).ok()?;
    let legacy_global_config = 'global: {
        let content = match os.fs.read(&legacy_global_config_path).await.ok() {
            Some(content) => content,
            None => break 'global None,
        };
        serde_json::from_slice::<ContextConfig>(&content).ok()
    };

    let mut create_hooks = None::<HashMap<String, Hook>>;
    let mut prompt_hooks = None::<HashMap<String, Hook>>;
    let mut included_files = None::<Vec<String>>;

    if let Some(config) = legacy_global_config {
        default_agent.included_files.extend(config.paths.clone());
        included_files = Some(config.paths);

        create_hooks = 'create_hooks: {
            if default_agent.create_hooks.is_array() {
                let existing_hooks = match serde_json::from_value::<Vec<String>>(default_agent.create_hooks.clone()) {
                    Ok(hooks) => hooks,
                    Err(_e) => break 'create_hooks None,
                };
                Some(existing_hooks.into_iter().enumerate().fold(
                    HashMap::<String, Hook>::new(),
                    |mut acc, (i, command)| {
                        acc.insert(
                            format!("start_hook_{i}"),
                            Hook::new_inline_hook(HookTrigger::ConversationStart, command),
                        );
                        acc
                    },
                ))
            } else {
                serde_json::from_value::<HashMap<String, Hook>>(default_agent.create_hooks.clone()).ok()
            }
        };

        prompt_hooks = 'prompt_hooks: {
            if default_agent.prompt_hooks.is_array() {
                let existing_hooks = match serde_json::from_value::<Vec<String>>(default_agent.prompt_hooks.clone()) {
                    Ok(hooks) => hooks,
                    Err(_e) => break 'prompt_hooks None,
                };
                Some(existing_hooks.into_iter().enumerate().fold(
                    HashMap::<String, Hook>::new(),
                    |mut acc, (i, command)| {
                        acc.insert(
                            format!("per_prompt_hook_{i}"),
                            Hook::new_inline_hook(HookTrigger::PerPrompt, command),
                        );
                        acc
                    },
                ))
            } else {
                serde_json::from_value::<HashMap<String, Hook>>(default_agent.prompt_hooks.clone()).ok()
            }
        };

        // We don't want to override anything in user's config
        // We need to return early if that is the case
        if let (Some(create_hooks), Some(prompt_hooks)) = (create_hooks.as_mut(), prompt_hooks.as_mut()) {
            for (name, hook) in config.hooks {
                match hook.trigger {
                    HookTrigger::ConversationStart => create_hooks.insert(name, hook),
                    HookTrigger::PerPrompt => prompt_hooks.insert(name, hook),
                };
            }
        } else {
            let _ = execute!(
                output,
                style::Print("Current default persona is malformed. Aborting migration.\n"),
                style::Print("Fix the default persona and try again")
            );
            return None;
        }
    }

    // At this point we can just unwrap the prompts and included files
    let mut create_hooks = create_hooks.unwrap_or_default();
    let mut prompt_hooks = prompt_hooks.unwrap_or_default();
    let mut included_files = included_files.unwrap_or_default();

    let legacy_profile_config_path = directories::chat_profiles_dir(os).ok()?;
    if !os.fs.exists(&legacy_profile_config_path) {
        return None;
    }

    let mut read_dir = os.fs.read_dir(&legacy_profile_config_path).await.ok()?;
    let mut profiles = HashMap::<String, ContextConfig>::new();

    // Here we assume every profile is stored under their own folders
    // And that the profile config is in profile_name/context.json
    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let config_file_path = entry.path().join("context.json");
        if !os.fs.exists(&config_file_path) {
            continue;
        }
        let profile_name = entry.file_name().to_str()?.to_string();
        let content = tokio::fs::read_to_string(&config_file_path).await.ok()?;
        let mut context_config = serde_json::from_str::<ContextConfig>(content.as_str()).ok()?;

        // Combine with global context since you can now only choose one agent at a time
        // So this is how we make what is previously global available to every new agent migrated
        context_config.paths.extend(included_files.clone());
        context_config.hooks.extend(create_hooks.clone());
        context_config.hooks.extend(prompt_hooks.clone());

        let back_up_path = entry.path().join("context.json.bak");
        if let Err(e) = os.fs.rename(config_file_path, back_up_path).await {
            let msg = format!("Failed to move legacy profile {profile_name} to back up: {e}");
            error!(msg);
            let _ = queue!(output, style::Print(msg),);
        }

        profiles.insert(profile_name, context_config);
    }

    let global_agent_path = directories::chat_global_persona_path(os).ok()?;
    let new_agents = profiles
        .into_iter()
        .fold(Vec::<Agent>::new(), |mut acc, (name, config)| {
            let (prompt_hooks_prime, create_hooks_prime) = config
                .hooks
                .into_iter()
                .partition::<HashMap<String, Hook>, _>(|(_, hook)| matches!(hook.trigger, HookTrigger::PerPrompt));

            // It could be the default profile that we are processing. If that's the case we should
            // just merge it with the default agent as opposed to creating a new one.
            if name.as_str() == "default" {
                prompt_hooks.extend(prompt_hooks_prime);
                create_hooks.extend(create_hooks_prime);
                included_files.extend(config.paths);
            } else {
                let prompt_hooks_prime = serde_json::to_value(prompt_hooks_prime);
                let create_hooks_prime = serde_json::to_value(create_hooks_prime);
                if let (Ok(prompt_hooks), Ok(create_hooks)) = (prompt_hooks_prime, create_hooks_prime) {
                    acc.push(Agent {
                        name: name.clone(),
                        path: Some(global_agent_path.join(format!("{name}.json"))),
                        included_files: config.paths,
                        prompt_hooks,
                        create_hooks,
                        ..Default::default()
                    });
                } else {
                    let msg = format!("Error serializing hooks for {name}. Skipping it for migration.");
                    let _ = queue!(output, style::Print(&msg));
                    error!(msg);
                }
            }
            acc
        });

    if !new_agents.is_empty() {
        let mut has_error = false;
        for new_agent in &new_agents {
            let Ok(content) = serde_json::to_string_pretty(new_agent) else {
                has_error = true;
                let _ = queue!(
                    output,
                    style::Print(format!(
                        "Failed to serialize profile {} for migration\n",
                        new_agent.name
                    )),
                    style::Print("Skipping")
                );
                continue;
            };
            let Some(config_path) = new_agent.path.as_ref() else {
                has_error = true;
                let _ = queue!(
                    output,
                    style::Print(format!(
                        "Failed to persist profile {} for migration: no path associated with new agent\n",
                        new_agent.name
                    )),
                    style::Print("Skipping")
                );
                continue;
            };
            if let Err(e) = os.fs.write(config_path, content.as_bytes()).await {
                has_error = true;
                let _ = queue!(
                    output,
                    style::Print(format!(
                        "Failed to persist profile {} for migration: {e}",
                        new_agent.name
                    )),
                    style::Print("Skipping")
                );
            }
        }

        if has_error {
            let _ = queue!(output, style::Print("One or more profile config has failed to migrate"),);
        }
    }

    // Finally we apply changes to the default agents and persist it accordingly
    if !create_hooks.is_empty() || !prompt_hooks.is_empty() || !included_files.is_empty() {
        default_agent.included_files.append(&mut included_files);

        match serde_json::to_value(create_hooks) {
            Ok(create_hooks) => {
                default_agent.create_hooks = create_hooks;
            },
            Err(e) => {
                error!("Error serializing create hooks for default agent: {:?}", e);
            },
        }

        match serde_json::to_value(prompt_hooks) {
            Ok(prompt_hooks) => default_agent.prompt_hooks = prompt_hooks,
            Err(e) => {
                error!("Error serializing prompt hooks for default agent: {:?}", e);
            },
        }

        if let Ok(content) = serde_json::to_string_pretty(default_agent) {
            let default_agent_path = default_agent.path.as_ref()?;
            os.fs.write(default_agent_path, content.as_bytes()).await.ok()?;
            let legacy_config_name = legacy_global_config_path.file_name()?.to_str()?;
            let back_up_path = legacy_global_config_path
                .parent()?
                .join(format!("{}.bak", legacy_config_name));
            os.fs.rename(&legacy_global_config_path, &back_up_path).await.ok()?;
        }
    }

    let _ = output.flush();

    Some(new_agents)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NullWriter;

    impl Write for NullWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    const INPUT: &str = r#"
            {
              "description": "My developer agent is used for small development tasks like solving open issues.",
              "prompt": "You are a principal developer who uses multiple agents to accomplish difficult engineering tasks",
              "mcpServers": {
                "fetch": { "command": "fetch3.1", "args": [] },
                "git": { "command": "git-mcp", "args": [] }
              },
              "tools": [                                    
                "@git",                                     
                "fs_read"
              ],
              "alias": {
                  "@gits/some_tool": "some_tool2"
              },
              "allowedTools": [                           
                "fs_read",                               
                "@fetch",
                "@gits/git_status"
              ],
              "includedFiles": [                        
                "~/my-genai-prompts/unittest.md"
              ],
              "createHooks": [                         
                "pwd && tree"
              ],
              "promptHooks": [                        
                "git status"
              ],
              "toolsSettings": {                     
                "fs_write": { "allowedPaths": ["~/**"] },
                "@git.git_status": { "git_user": "$GIT_USER" }
              }
            }
        "#;

    #[test]
    fn test_deser() {
        let agent = serde_json::from_str::<Agent>(INPUT).expect("Deserializtion failed");
        assert!(agent.mcp_servers.mcp_servers.contains_key("fetch"));
        assert!(agent.mcp_servers.mcp_servers.contains_key("git"));
        assert!(agent.alias.contains_key("@gits/some_tool"));
    }

    #[test]
    fn test_get_active() {
        let mut collection = Agents::default();
        assert!(collection.get_active().is_none());

        let agent = Agent::default();
        collection.agents.insert("default".to_string(), agent);
        collection.active_idx = "default".to_string();

        assert!(collection.get_active().is_some());
        assert_eq!(collection.get_active().unwrap().name, "default");
    }

    #[test]
    fn test_get_active_mut() {
        let mut collection = Agents::default();
        assert!(collection.get_active_mut().is_none());

        let agent = Agent::default();
        collection.agents.insert("default".to_string(), agent);
        collection.active_idx = "default".to_string();

        assert!(collection.get_active_mut().is_some());
        let active = collection.get_active_mut().unwrap();
        active.description = Some("Modified description".to_string());

        assert_eq!(
            collection.agents.get("default").unwrap().description,
            Some("Modified description".to_string())
        );
    }

    #[test]
    fn test_switch() {
        let mut collection = Agents::default();

        let default_agent = Agent::default();
        let dev_agent = Agent {
            name: "dev".to_string(),
            description: Some("Developer agent".to_string()),
            ..Default::default()
        };

        collection.agents.insert("default".to_string(), default_agent);
        collection.agents.insert("dev".to_string(), dev_agent);
        collection.active_idx = "default".to_string();

        // Test successful switch
        let result = collection.switch("dev");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().name, "dev");

        // Test switch to non-existent agent
        let result = collection.switch("nonexistent");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "No agent with name nonexistent found");
    }

    #[tokio::test]
    async fn test_list_personas() {
        let mut collection = Agents::default();

        // Add two agents
        let default_agent = Agent::default();
        let dev_agent = Agent {
            name: "dev".to_string(),
            description: Some("Developer agent".to_string()),
            ..Default::default()
        };

        collection.agents.insert("default".to_string(), default_agent);
        collection.agents.insert("dev".to_string(), dev_agent);

        let result = collection.list_personas();
        assert!(result.is_ok());

        let personas = result.unwrap();
        assert_eq!(personas.len(), 2);
        assert!(personas.contains(&"default".to_string()));
        assert!(personas.contains(&"dev".to_string()));
    }

    #[tokio::test]
    async fn test_create_persona() {
        let mut collection = Agents::default();
        let ctx = Os::new().await.unwrap();

        let persona_name = "test_persona";
        let result = collection.create_persona(&ctx, persona_name).await;
        assert!(result.is_ok());
        let persona_path = directories::chat_global_persona_path(&ctx)
            .expect("Error obtaining global persona path")
            .join(format!("{persona_name}.json"));
        assert!(persona_path.exists());
        assert!(collection.agents.contains_key(persona_name));

        // Test with creating a persona with the same name
        let result = collection.create_persona(&ctx, persona_name).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            format!("Persona '{persona_name}' already exists")
        );

        // Test invalid persona names
        let result = collection.create_persona(&ctx, "").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Persona name cannot be empty");

        let result = collection.create_persona(&ctx, "123-invalid!").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_persona() {
        let mut collection = Agents::default();
        let ctx = Os::new().await.unwrap();

        let persona_name_one = "test_persona_one";
        collection
            .create_persona(&ctx, persona_name_one)
            .await
            .expect("Failed to create persona");
        let persona_name_two = "test_persona_two";
        collection
            .create_persona(&ctx, persona_name_two)
            .await
            .expect("Failed to create persona");

        collection.switch(persona_name_one).expect("Failed to switch persona");

        // Should not be able to delete active persona
        let active = collection
            .get_active()
            .expect("Failed to obtain active persona")
            .name
            .clone();
        let result = collection.delete_persona(&ctx, &active).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Cannot delete the active persona. Switch to another persona first"
        );

        // Should be able to delete inactive persona
        let persona_two_path = collection
            .agents
            .get(persona_name_two)
            .expect("Failed to obtain persona that's yet to be deleted")
            .path
            .clone()
            .expect("Persona should have path");
        let result = collection.delete_persona(&ctx, persona_name_two).await;
        assert!(result.is_ok());
        assert!(!collection.agents.contains_key(persona_name_two));
        assert!(!persona_two_path.exists());

        let result = collection.delete_persona(&ctx, "nonexistent").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "Persona 'nonexistent' does not exist");
    }

    #[test]
    fn test_validate_persona_name() {
        // Valid names
        assert!(validate_persona_name("valid").is_ok());
        assert!(validate_persona_name("valid123").is_ok());
        assert!(validate_persona_name("valid-name").is_ok());
        assert!(validate_persona_name("valid_name").is_ok());
        assert!(validate_persona_name("123valid").is_ok());

        // Invalid names
        assert!(validate_persona_name("").is_err());
        assert!(validate_persona_name("-invalid").is_err());
        assert!(validate_persona_name("_invalid").is_err());
        assert!(validate_persona_name("invalid!").is_err());
        assert!(validate_persona_name("invalid space").is_err());
    }
}
