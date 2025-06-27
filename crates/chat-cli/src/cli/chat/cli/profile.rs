use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

use clap::Subcommand;
use crossterm::style::{
    self,
    Attribute,
    Color,
};
use crossterm::{
    execute,
    queue,
};
use tracing::error;

use crate::cli::agent::Agent;
use crate::cli::chat::cli::hooks::{
    Hook,
    HookTrigger,
};
use crate::cli::chat::context::ContextConfig;
use crate::cli::chat::{
    ChatError,
    ChatSession,
    ChatState,
};
use crate::os::Os;
use crate::util::directories::{
    self,
    chat_global_persona_path,
};

#[deny(missing_docs)]
#[derive(Debug, PartialEq, Subcommand)]
#[command(
    before_long_help = "Profiles allow you to organize and manage different sets of context files for different projects or tasks.

Notes
• The \"global\" profile contains context files that are available in all profiles
• The \"default\" profile is used when no profile is specified
• You can switch between profiles to work on different projects
• Each profile maintains its own set of context files"
)]
pub enum ProfileSubcommand {
    /// List all available profiles
    List,
    /// Create a new profile with the specified name
    Create { name: String },
    /// Delete the specified profile
    Delete { name: String },
    /// Switch to the specified profile
    Set { name: String },
    /// Rename a profile
    Rename { old_name: String, new_name: String },
    /// Migrate existing profiles to persona
    Migrate,
}

impl ProfileSubcommand {
    pub async fn execute(self, os: &Os, session: &mut ChatSession) -> Result<ChatState, ChatError> {
        let agents = &session.conversation.agents;

        macro_rules! _print_err {
            ($err:expr) => {
                execute!(
                    session.stderr,
                    style::SetForegroundColor(Color::Red),
                    style::Print(format!("\nError: {}\n\n", $err)),
                    style::SetForegroundColor(Color::Reset)
                )?
            };
        }

        match self {
            Self::List => {
                let profiles = agents.agents.values().collect::<Vec<_>>();
                let active_profile = agents.get_active();

                execute!(session.stderr, style::Print("\n"))?;
                for profile in profiles {
                    if active_profile.is_some_and(|p| p == profile) {
                        execute!(
                            session.stderr,
                            style::SetForegroundColor(Color::Green),
                            style::Print("* "),
                            style::Print(&profile.name),
                            style::SetForegroundColor(Color::Reset),
                            style::Print("\n")
                        )?;
                    } else {
                        execute!(
                            session.stderr,
                            style::Print("  "),
                            style::Print(&profile.name),
                            style::Print("\n")
                        )?;
                    }
                }
                execute!(session.stderr, style::Print("\n"))?;
            },
            Self::Migrate => {
                let legacy_profile_config_path = directories::chat_profiles_dir(os).map_err(|e| {
                    ChatError::Custom(format!("Error retrieving chat profile dir for migration: {e}").into())
                })?;
                if !os.fs.exists(&legacy_profile_config_path) {
                    return Err(ChatError::Custom(
                        "No legacy profile directory detected. Aborting\n".into(),
                    ));
                }
                let profile_backup_path = legacy_profile_config_path
                    .parent()
                    .ok_or(ChatError::Custom(
                        "Migration failed due to failure to find legacy profile directory parent\n".into(),
                    ))?
                    .join("profiles.bak");
                if os.fs.exists(&profile_backup_path) {
                    return Err(ChatError::Custom(
                        format!(
                            "Previous backup detected. Delete {} and try again\n",
                            profile_backup_path.to_string_lossy()
                        )
                        .into(),
                    ));
                }

                let (_, default_agent) = session
                    .conversation
                    .agents
                    .agents
                    .iter_mut()
                    .find(|(name, _agent)| name.as_str() == "default")
                    .ok_or(ChatError::Custom("Failed to obtain default agent".into()))?;

                let mut default_ch = 'create_hooks: {
                    if default_agent.create_hooks.is_array() {
                        let existing_hooks =
                            match serde_json::from_value::<Vec<String>>(default_agent.create_hooks.clone()) {
                                Ok(hooks) => hooks,
                                Err(_e) => break 'create_hooks None::<HashMap<String, Hook>>,
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
                }
                .unwrap_or_default();

                let mut default_ph = 'prompt_hooks: {
                    if default_agent.prompt_hooks.is_array() {
                        let existing_hooks =
                            match serde_json::from_value::<Vec<String>>(default_agent.prompt_hooks.clone()) {
                                Ok(hooks) => hooks,
                                Err(_e) => break 'prompt_hooks None::<HashMap<String, Hook>>,
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
                }
                .unwrap_or_default();

                let default_files = &mut default_agent.included_files;

                if !os.fs.exists(&legacy_profile_config_path) {
                    return Err(ChatError::Custom(
                        "No legacy profile detected. Aborting migration.".into(),
                    ));
                }

                let mut read_dir = os.fs.read_dir(&legacy_profile_config_path).await?;
                let mut profiles = HashMap::<String, ContextConfig>::new();
                let mut has_default_profile = false;

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
                    context_config.paths.extend(default_files.clone());
                    context_config.hooks.extend(default_ch.clone());
                    context_config.hooks.extend(default_ph.clone());

                    profiles.insert(profile_name.clone(), context_config);
                }

                let global_agent_path = directories::chat_global_persona_path(os).map_err(|e| {
                    ChatError::Custom(format!("Failed to obtain global persona path for migration {e}").into())
                })?;
                let new_agents = profiles
                    .into_iter()
                    .fold(Vec::<Agent>::new(), |mut acc, (name, config)| {
                        let (prompt_hooks_prime, create_hooks_prime) = config
                            .hooks
                            .into_iter()
                            .partition::<HashMap<String, Hook>, _>(|(_, hook)| {
                                matches!(hook.trigger, HookTrigger::PerPrompt)
                            });

                        // It could be the default profile that we are processing. If that's the case we should
                        // just merge it with the default agent as opposed to creating a new one.
                        if name.as_str() == "default" {
                            has_default_profile = true;
                            default_ph.extend(prompt_hooks_prime);
                            default_ch.extend(create_hooks_prime);
                            default_files.extend(config.paths);
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
                                let _ = queue!(session.stderr, style::Print(&msg));
                                error!(msg);
                            }
                        }
                        acc
                    });

                let mut legacy_backup_path = None::<PathBuf>;
                if !new_agents.is_empty() || has_default_profile {
                    let mut has_error = false;
                    for new_agent in &new_agents {
                        let Ok(content) = serde_json::to_string_pretty(new_agent) else {
                            has_error = true;
                            queue!(
                                session.stderr,
                                style::Print(format!(
                                    "Failed to serialize profile {} for migration\n",
                                    new_agent.name
                                )),
                                style::Print("Skipping\n")
                            )?;
                            continue;
                        };
                        let Some(config_path) = new_agent.path.as_ref() else {
                            has_error = true;
                            queue!(
                                session.stderr,
                                style::Print(format!(
                                    "Failed to persist profile {} for migration: no path associated with new agent\n",
                                    new_agent.name
                                )),
                                style::Print("Skipping\n")
                            )?;
                            continue;
                        };
                        if let Err(e) = os.fs.write(config_path, content.as_bytes()).await {
                            has_error = true;
                            queue!(
                                session.stderr,
                                style::Print(format!(
                                    "Failed to persist profile {} for migration: {e}",
                                    new_agent.name
                                )),
                                style::Print("Skipping\n")
                            )?;
                        }
                    }

                    // Here we are moving / renaming the /profiles directory to /profiles.bak
                    // This is how we ensure we don't prompt users to run profile migratios if they
                    // have already successfully migrated
                    if has_error {
                        queue!(
                            session.stderr,
                            style::Print("One or more profile config has failed to migrate"),
                        )?;
                    } else if let Some(profile_backup_path) = legacy_profile_config_path.parent() {
                        let profile_backup_path = profile_backup_path.join("profiles.bak");
                        if let Err(e) = os.fs.rename(&legacy_profile_config_path, &profile_backup_path).await {
                            queue!(
                                session.stderr,
                                style::Print(format!("Renaming of legacy profile directory failed: {e}\n")),
                                style::Print(
                                    "Please delete the legacy profile directory to avoid being prompted to migrate in future"
                                )
                            )?;
                        }
                        legacy_backup_path.replace(profile_backup_path);
                    } else {
                        queue!(
                            session.stderr,
                            style::Print(
                                "Renaming of legacy profile directory failed due to failure to find directory parent\n"
                            ),
                            style::Print(
                                "Please delete the legacy profile directory to avoid being prompted to migrate in future"
                            )
                        )?;
                    }
                }

                // Finally we apply changes to the default agents and persist it accordingly
                if has_default_profile {
                    match serde_json::to_value(default_ch) {
                        Ok(create_hooks) => {
                            default_agent.create_hooks = create_hooks;
                        },
                        Err(e) => {
                            error!("Error serializing create hooks for default agent: {:?}", e);
                        },
                    }

                    match serde_json::to_value(default_ph) {
                        Ok(prompt_hooks) => default_agent.prompt_hooks = prompt_hooks,
                        Err(e) => {
                            error!("Error serializing prompt hooks for default agent: {:?}", e);
                        },
                    }

                    if let Ok(content) = serde_json::to_string_pretty(default_agent) {
                        let default_agent_path = default_agent.path.as_ref().ok_or(ChatError::Custom(
                                "Profile migration failed for default profile because default agent does not have a path associated".into()
                        ))?;
                        os.fs.write(default_agent_path, content.as_bytes()).await.map_err(|e| {
                            ChatError::Custom(format!("Profile migration failed to persist: {e}").into())
                        })?;
                        error!("## perm: default profile persisted");
                    }
                }

                if let Some(backup_path) = legacy_backup_path {
                    queue!(
                        session.stderr,
                        style::Print(format!(
                            "Profile migration completed. Old profiles can be found at {}\n",
                            backup_path.to_string_lossy()
                        )),
                        style::Print(format!(
                            "Note that the migration simply created new config under {}. If these profiles contain context that references files under this path, you would need to edit them accordingly in the new config",
                            global_agent_path.to_string_lossy()
                        ))
                    )?;
                }

                session.stderr.flush()?;
            },
            Self::Rename { .. } | Self::Set { .. } | Self::Delete { .. } | Self::Create { .. } => {
                // As part of the persona implementation, we are disabling the ability to
                // switch / create profile after a session has started.
                // TODO: perhaps revive this after we have a decision on profile create /
                // switch
                let global_path = if let Ok(path) = chat_global_persona_path(os) {
                    path.to_str().unwrap_or("default global persona path").to_string()
                } else {
                    "default global persona path".to_string()
                };
                execute!(
                    session.stderr,
                    style::SetForegroundColor(Color::Yellow),
                    style::Print(format!(
                        "Persona / Profile persistence has been disabled. To persist any changes on persona / profile, use the default persona under {} as example",
                        global_path
                    )),
                    style::SetAttribute(Attribute::Reset)
                )?;
            },
        }

        Ok(ChatState::PromptUser {
            skip_printing_tools: true,
        })
    }
}
