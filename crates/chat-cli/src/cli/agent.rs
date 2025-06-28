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
    queue,
    style,
};
use dialoguer::Select;
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
use crate::database::settings::Setting;
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
    /// Agent names are derived from the file name. Thus they are skipped for
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
            let local_config_dir = directories::chat_local_agent_dir()?.join(agent_name);
            if os.fs.exists(&local_config_dir) {
                break 'config Ok::<PathBuf, PathBuf>(local_config_dir);
            }

            let global_config_dir = directories::chat_global_agent_path(os)?.join(format!("{agent_name}.json"));
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

    /// Migrated from [reload_profiles] from context.rs. It loads the active agent from disk and
    /// replaces its in-memory counterpart with it.
    pub async fn reload_agents(&mut self, os: &Os, output: &mut impl Write) -> eyre::Result<()> {
        let persona_name = self.get_active().map(|a| a.name.as_str());
        let mut new_self = Self::load(os, persona_name, output).await;
        std::mem::swap(self, &mut new_self);
        Ok(())
    }

    pub fn list_agents(&self) -> eyre::Result<Vec<String>> {
        Ok(self.agents.keys().cloned().collect::<Vec<_>>())
    }

    /// Migrated from [create_profile] from context.rs, which was creating profiles under the
    /// global directory. We shall preserve this implicit behavior for now until further notice.
    pub async fn create_agent(&mut self, os: &Os, name: &str) -> eyre::Result<()> {
        validate_agent_name(name)?;

        let agent_path = directories::chat_global_agent_path(os)?.join(format!("{name}.json"));
        if agent_path.exists() {
            return Err(eyre::eyre!("Agent '{}' already exists", name));
        }

        let agent = Agent {
            name: name.to_string(),
            path: Some(agent_path.clone()),
            ..Default::default()
        };
        let contents = serde_json::to_string_pretty(&agent)
            .map_err(|e| eyre::eyre!("Failed to serialize profile configuration: {}", e))?;

        if let Some(parent) = agent_path.parent() {
            os.fs.create_dir_all(parent).await?;
        }
        os.fs.write(&agent_path, contents).await?;

        self.agents.insert(name.to_string(), agent);

        Ok(())
    }

    /// Migrated from [delete_profile] from context.rs, which was deleting profiles under the
    /// global directory. We shall preserve this implicit behavior for now until further notice.
    pub async fn delete_agent(&mut self, os: &Os, name: &str) -> eyre::Result<()> {
        if name == self.active_idx.as_str() {
            eyre::bail!("Cannot delete the active agent. Switch to another agent first");
        }

        let to_delete = self
            .agents
            .get(name)
            .ok_or(eyre::eyre!("Agent '{name}' does not exist"))?;
        match to_delete.path.as_ref() {
            Some(path) if path.exists() => {
                os.fs.remove_file(path).await?;
            },
            _ => eyre::bail!("Agent {name} does not have an associated path"),
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
            let Ok(path) = directories::chat_local_agent_dir() else {
                break 'local Vec::<Agent>::new();
            };
            let Ok(files) = tokio::fs::read_dir(path).await else {
                break 'local Vec::<Agent>::new();
            };
            load_agents_from_entries(files).await
        };

        let mut global_agents = 'global: {
            let Ok(path) = directories::chat_global_agent_path(os) else {
                break 'global Vec::<Agent>::new();
            };
            let files = match tokio::fs::read_dir(&path).await {
                Ok(files) => files,
                Err(e) => {
                    if matches!(e.kind(), io::ErrorKind::NotFound) {
                        if let Err(e) = os.fs.create_dir_all(&path).await {
                            error!("Error creating global agent dir: {:?}", e);
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
                    style::Print("Agent conflict for "),
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

        local_agents.append(&mut global_agents);

        // Ensure that we always have a default agent under the global directory
        if !local_agents.iter().any(|a| a.name == "default") {
            let default_agent = Agent {
                path: directories::chat_global_agent_path(os)
                    .ok()
                    .map(|p| p.join("default.json")),
                ..Default::default()
            };

            match serde_json::to_string_pretty(&default_agent) {
                Ok(content) => {
                    if let Ok(path) = directories::chat_global_agent_path(os) {
                        let default_path = path.join("default.json");
                        if let Err(e) = tokio::fs::write(default_path, &content).await {
                            error!("Error writing default agent to file: {:?}", e);
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

        match migrate_global_context(os, default_agent).await {
            Ok(true) => {
                let _ = queue!(
                    output,
                    style::Print(format!(
                        "Global context config has been migrated to {}\n",
                        default_agent
                            .path
                            .as_ref()
                            .and_then(|p| p.to_str())
                            .unwrap_or("global config directory.")
                    ))
                );
            },
            Ok(false) => {},
            Err(e) => {
                let _ = queue!(
                    output,
                    style::Print(format!(
                        "Current default agent is malformed: {e}\nAborting migration.\n"
                    )),
                    style::Print("Fix the default agent and try again")
                );
            },
        }

        // Check to see if we have legacy profile directory still
        if let Ok(_legacy_profile_dir) = directories::chat_profiles_dir(os) {
            let _ = queue!(
                output,
                style::Print("Legacy profile directory detected. Run /profile migrate to migrate them")
            );
        }

        let _ = output.flush();

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

struct ContextMigrate<const S: char> {
    legacy_global_context: Option<ContextConfig>,
    legacy_profiles: HashMap<String, ContextConfig>,
    new_agents: Vec<Agent>,
}

impl ContextMigrate<'a'> {
    async fn scan(os: &Os) -> eyre::Result<ContextMigrate<'b'>> {
        let legacy_global_context_path = directories::chat_global_context_path(os)?;
        let legacy_global_context: Option<ContextConfig> = 'global: {
            let Ok(content) = os.fs.read(&legacy_global_context_path).await else {
                break 'global None;
            };
            serde_json::from_slice::<ContextConfig>(&content).ok()
        };

        let legacy_profile_path = directories::chat_profiles_dir(os)?;
        let legacy_profiles: HashMap<String, ContextConfig> = 'profiles: {
            let mut profiles = HashMap::<String, ContextConfig>::new();
            let Ok(mut read_dir) = os.fs.read_dir(&legacy_profile_path).await else {
                break 'profiles profiles;
            };

            // Here we assume every profile is stored under their own folders
            // And that the profile config is in profile_name/context.json
            while let Ok(Some(entry)) = read_dir.next_entry().await {
                let config_file_path = entry.path().join("context.json");
                if !os.fs.exists(&config_file_path) {
                    continue;
                }
                let Some(profile_name) = entry.file_name().to_str().map(|s| s.to_string()) else {
                    continue;
                };
                let Ok(content) = tokio::fs::read_to_string(&config_file_path).await else {
                    continue;
                };
                let Ok(mut context_config) = serde_json::from_str::<ContextConfig>(content.as_str()) else {
                    continue;
                };

                // Combine with global context since you can now only choose one agent at a time
                // So this is how we make what is previously global available to every new agent migrated
                if let Some(context) = legacy_global_context.as_ref() {
                    context_config.paths.extend(context.paths.clone());
                    context_config.hooks.extend(context.hooks.clone());
                }

                profiles.insert(profile_name.clone(), context_config);
            }

            profiles
        };

        if legacy_global_context.is_some() || !legacy_profiles.is_empty() {
            Ok(ContextMigrate {
                legacy_global_context,
                legacy_profiles,
                new_agents: vec![],
            })
        } else {
            bail!("Nothing to migrate");
        }
    }
}

impl ContextMigrate<'b'> {
    async fn prompt_migrate(self) -> eyre::Result<ContextMigrate<'c'>> {
        let ContextMigrate {
            legacy_global_context,
            legacy_profiles,
            new_agents,
        } = self;

        let labels = vec!["Yes", "No"];
        let selection: Option<_> = match Select::with_theme(&crate::util::dialoguer_theme())
            .with_prompt(
                "You have context and/or profiles that belong to a legacy config. Would you like to migrate them?",
            )
            .items(&labels)
            .default(1)
            .interact_on_opt(&dialoguer::console::Term::stdout())
        {
            Ok(sel) => {
                let _ = crossterm::execute!(
                    std::io::stdout(),
                    crossterm::style::SetForegroundColor(crossterm::style::Color::Magenta)
                );
                sel
            },
            // Ctrl‑C -> Err(Interrupted)
            Err(dialoguer::Error::IO(ref e)) if e.kind() == std::io::ErrorKind::Interrupted => None,
            Err(e) => bail!("Failed to choose an option: {e}"),
        };

        if let Some(0) = selection {
            Ok(ContextMigrate {
                legacy_global_context,
                legacy_profiles,
                new_agents,
            })
        } else {
            bail!("Aborting migration")
        }
    }
}

impl ContextMigrate<'c'> {
    async fn migrate(self, os: &Os) -> eyre::Result<ContextMigrate<'d'>> {
        let ContextMigrate {
            legacy_global_context,
            legacy_profiles,
            new_agents,
        } = self;

        // Migration of global context
        if let Some(context) = &legacy_global_context {}

        // Migration of profile context

        Ok(ContextMigrate {
            legacy_global_context: None,
            legacy_profiles,
            new_agents,
        })
    }
}

impl ContextMigrate<'d'> {
    async fn prompt_set_default(self, os: &mut Os) -> eyre::Result<(Option<usize>, Vec<Agent>)> {
        let ContextMigrate { new_agents, .. } = self;

        let labels = new_agents.iter().map(|a| a.name.as_str()).collect::<Vec<_>>();
        let selection: Option<_> = match Select::with_theme(&crate::util::dialoguer_theme())
            .with_prompt(
                "Set an agent as default. This is the agent that q chat will launch with unless specified otherwise.",
            )
            .items(&labels)
            .interact_on_opt(&dialoguer::console::Term::stdout())
        {
            Ok(sel) => {
                let _ = crossterm::execute!(
                    std::io::stdout(),
                    crossterm::style::SetForegroundColor(crossterm::style::Color::Magenta)
                );
                sel
            },
            // Ctrl‑C -> Err(Interrupted)
            Err(dialoguer::Error::IO(ref e)) if e.kind() == std::io::ErrorKind::Interrupted => None,
            Err(e) => bail!("Failed to choose an option: {e}"),
        };

        let mut agent_to_load = None::<usize>;
        if let Some(i) = selection {
            if let Some(name) = labels.get(i) {
                if let Ok(value) = serde_json::to_value(name) {
                    if os.database.settings.set(Setting::ChatDefaultAgent, value).await.is_ok() {
                        agent_to_load.replace(i);
                    }
                }
            }
        }

        Ok((agent_to_load, new_agents))
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
                    tracing::error!("Error reading agent file {file_path}: {:?}", e);
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
                    tracing::error!("Error deserializing agent file {file_path}: {:?}", e);
                    continue;
                },
            };
            if let Some(name) = Path::new(&file.file_name()).file_stem() {
                agent.name = name.to_string_lossy().to_string();
                res.push(agent);
            } else {
                let file_path = file_path.to_string_lossy();
                tracing::error!("Unable to determine agent name from config file at {file_path}, skipping");
            }
        }
    }
    res
}

fn validate_agent_name(name: &str) -> eyre::Result<()> {
    // Check if name is empty
    if name.is_empty() {
        eyre::bail!("Agent name cannot be empty");
    }

    // Check if name contains only allowed characters and starts with an alphanumeric character
    let re = Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9_-]*$")?;
    if !re.is_match(name) {
        eyre::bail!(
            "Agent name must start with an alphanumeric character and can only contain alphanumeric characters, hyphens, and underscores"
        );
    }

    Ok(())
}

async fn migrate(os: &mut Os) -> eyre::Result<(Option<usize>, Vec<Agent>)> {
    ContextMigrate::<'a'>::scan(os)
        .await?
        .prompt_migrate()
        .await?
        .migrate(os)
        .await?
        .prompt_set_default(os)
        .await
}

/// Migration of context consists of the following:
/// 1. Scan for global context config.
/// 2. If it does not exist. Signal to the caller that no migration was done.
/// 3. If it does, deserialize the legacy global config and merge it with the default agent, follow
///    by persisting it on disk.
async fn migrate_global_context(os: &Os, default_agent: &mut Agent) -> eyre::Result<bool> {
    let legacy_global_config_path = directories::chat_global_context_path(os)?;
    if !os.fs.exists(&legacy_global_config_path) {
        return Ok(false);
    }
    let legacy_global_config = {
        let content = os.fs.read(&legacy_global_config_path).await?;
        serde_json::from_slice::<ContextConfig>(&content)?
    };

    default_agent.included_files.extend(legacy_global_config.paths);

    let mut create_hooks = {
        if default_agent.create_hooks.is_array() {
            let existing_hooks = serde_json::from_value::<Vec<String>>(default_agent.create_hooks.clone())?;
            existing_hooks
                .into_iter()
                .enumerate()
                .fold(HashMap::<String, Hook>::new(), |mut acc, (i, command)| {
                    acc.insert(
                        format!("start_hook_{i}"),
                        Hook::new_inline_hook(HookTrigger::ConversationStart, command),
                    );
                    acc
                })
        } else {
            serde_json::from_value::<HashMap<String, Hook>>(default_agent.create_hooks.clone())?
        }
    };

    let mut prompt_hooks = {
        if default_agent.prompt_hooks.is_array() {
            let existing_hooks = serde_json::from_value::<Vec<String>>(default_agent.prompt_hooks.clone())?;
            existing_hooks
                .into_iter()
                .enumerate()
                .fold(HashMap::<String, Hook>::new(), |mut acc, (i, command)| {
                    acc.insert(
                        format!("per_prompt_hook_{i}"),
                        Hook::new_inline_hook(HookTrigger::PerPrompt, command),
                    );
                    acc
                })
        } else {
            serde_json::from_value::<HashMap<String, Hook>>(default_agent.prompt_hooks.clone())?
        }
    };

    // We don't want to override anything in user's config
    // We need to return early if that is the case
    for (name, hook) in legacy_global_config.hooks {
        match hook.trigger {
            HookTrigger::ConversationStart => create_hooks.insert(name, hook),
            HookTrigger::PerPrompt => prompt_hooks.insert(name, hook),
        };
    }

    let content = serde_json::to_string_pretty(default_agent)?;
    let path = default_agent.path.as_ref().ok_or(eyre::eyre!(
        "Failed to persist default agent. Associated path not found."
    ))?;
    os.fs.write(path, content.as_bytes()).await?;
    let global_context_backup_path = legacy_global_config_path
        .parent()
        .ok_or(eyre::eyre!(
            "Failed to persist default agent. Parent folder directory not found."
        ))?
        .join(format!(
            "{}.bak",
            legacy_global_config_path
                .file_name()
                .ok_or(eyre::eyre!(
                    "Failed to persist default agent. Error retrieving legacy file name."
                ))?
                .to_string_lossy()
        ));
    os.fs
        .rename(&legacy_global_config_path, global_context_backup_path)
        .await?;

    Ok(true)
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
    async fn test_list_agents() {
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

        let result = collection.list_agents();
        assert!(result.is_ok());

        let agents = result.unwrap();
        assert_eq!(agents.len(), 2);
        assert!(agents.contains(&"default".to_string()));
        assert!(agents.contains(&"dev".to_string()));
    }

    #[tokio::test]
    async fn test_create_agent() {
        let mut collection = Agents::default();
        let ctx = Os::new().await.unwrap();

        let agent_name = "test_agent";
        let result = collection.create_agent(&ctx, agent_name).await;
        assert!(result.is_ok());
        let agent_path = directories::chat_global_agent_path(&ctx)
            .expect("Error obtaining global agent path")
            .join(format!("{agent_name}.json"));
        assert!(agent_path.exists());
        assert!(collection.agents.contains_key(agent_name));

        // Test with creating a agent with the same name
        let result = collection.create_agent(&ctx, agent_name).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            format!("agent '{agent_name}' already exists")
        );

        // Test invalid agent names
        let result = collection.create_agent(&ctx, "").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "agent name cannot be empty");

        let result = collection.create_agent(&ctx, "123-invalid!").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_agent() {
        let mut collection = Agents::default();
        let ctx = Os::new().await.unwrap();

        let agent_name_one = "test_agent_one";
        collection
            .create_agent(&ctx, agent_name_one)
            .await
            .expect("Failed to create agent");
        let agent_name_two = "test_agent_two";
        collection
            .create_agent(&ctx, agent_name_two)
            .await
            .expect("Failed to create agent");

        collection.switch(agent_name_one).expect("Failed to switch agent");

        // Should not be able to delete active agent
        let active = collection
            .get_active()
            .expect("Failed to obtain active agent")
            .name
            .clone();
        let result = collection.delete_agent(&ctx, &active).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Cannot delete the active agent. Switch to another agent first"
        );

        // Should be able to delete inactive agent
        let agent_two_path = collection
            .agents
            .get(agent_name_two)
            .expect("Failed to obtain agent that's yet to be deleted")
            .path
            .clone()
            .expect("agent should have path");
        let result = collection.delete_agent(&ctx, agent_name_two).await;
        assert!(result.is_ok());
        assert!(!collection.agents.contains_key(agent_name_two));
        assert!(!agent_two_path.exists());

        let result = collection.delete_agent(&ctx, "nonexistent").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().to_string(), "agent 'nonexistent' does not exist");
    }

    #[test]
    fn test_validate_agent_name() {
        // Valid names
        assert!(validate_agent_name("valid").is_ok());
        assert!(validate_agent_name("valid123").is_ok());
        assert!(validate_agent_name("valid-name").is_ok());
        assert!(validate_agent_name("valid_name").is_ok());
        assert!(validate_agent_name("123valid").is_ok());

        // Invalid names
        assert!(validate_agent_name("").is_err());
        assert!(validate_agent_name("-invalid").is_err());
        assert!(validate_agent_name("_invalid").is_err());
        assert!(validate_agent_name("invalid!").is_err());
        assert!(validate_agent_name("invalid space").is_err());
    }
}
