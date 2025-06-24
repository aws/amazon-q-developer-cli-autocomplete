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
    pub async fn load(os: &Os, persona_name: Option<&str>, output: &mut impl Write) -> Self {
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

        Self {
            agents: local_agents
                .into_iter()
                .map(|a| (a.name.clone(), a))
                .collect::<HashMap<_, _>>(),
            active_idx: persona_name.unwrap_or("default").to_string(),
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
