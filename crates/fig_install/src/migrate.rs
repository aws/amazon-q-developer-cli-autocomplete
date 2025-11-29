use std::path::PathBuf;

use eyre::Result;
use fig_integrations::shell::ShellExt;
use fig_os_shim::Context;
use fig_util::Shell;
use rustix::fs::{
    FlockOperation,
    flock,
};
use serde_json::Value;
use tokio::fs;
use tracing::{
    debug,
    warn,
};

const KIRO_MIGRATION_KEY: &str = "migration.kiro.completed";
const MIGRATION_TIMEOUT_SECS: u64 = 60;

pub async fn migrate_if_needed() -> Result<bool> {
    let start = std::time::Instant::now();
    let status = detect_migration().await?;

    match status {
        MigrationStatus::Completed => {
            debug!("Migration already completed");
            return Ok(false);
        },
        MigrationStatus::NotNeeded => {
            debug!("No migration needed");
            mark_migration_completed()?;
            return Ok(false);
        },
        MigrationStatus::Needed => {
            debug!("Migrating database and settings");
        },
    }

    let _lock = match acquire_migration_lock()? {
        Some(lock) => lock,
        None => {
            debug!("Migration already in progress");
            return Ok(false);
        },
    };

    let old_dir = fig_util::directories::old_fig_data_dir()?;
    let new_dir = fig_util::directories::fig_data_dir()?;

    debug!("Old directory: {}", old_dir.display());
    debug!("New directory: {}", new_dir.display());

    // Copy essential files from old directory to new directory
    if !new_dir.exists() {
        fs::create_dir_all(&new_dir).await?;
    }
    debug!("Copying essential files from old to new directory");
    copy_essential_files(&old_dir, &new_dir).await?;

    debug!("Migrating shell integrations");
    let errors = migrate_dotfiles().await;
    if !errors.is_empty() {
        warn!(?errors, "errors occurred migrating shell integrations");
    }

    // Mark migration as completed in database
    debug!("Marking migration as completed");
    mark_migration_completed()?;

    debug!("Migration completed successfully in {:?}", start.elapsed());
    Ok(true)
}

#[derive(Debug)]
enum MigrationStatus {
    NotNeeded,
    Needed,
    Completed,
}

async fn detect_migration() -> Result<MigrationStatus> {
    let old_dir = fig_util::directories::old_fig_data_dir()?;
    let new_dir = fig_util::directories::fig_data_dir()?;

    // If new directory doesn't exist yet, check if old directory exists
    if !new_dir.exists() {
        if old_dir.exists() && old_dir.is_dir() {
            return Ok(MigrationStatus::Needed);
        } else {
            return Ok(MigrationStatus::NotNeeded);
        }
    }

    // New directory exists, check database flag (safe now since new_dir exists)
    let migration_completed = is_migration_completed()?;

    if migration_completed {
        Ok(MigrationStatus::Completed)
    } else if old_dir.exists() && old_dir.is_dir() {
        Ok(MigrationStatus::Needed)
    } else {
        Ok(MigrationStatus::NotNeeded)
    }
}

async fn copy_essential_files(src: &std::path::Path, dst: &std::path::Path) -> Result<()> {
    // Copy data-local-dir files (database and history)
    let data_files = ["data.sqlite3", "history"];
    for file_name in data_files {
        let src_path = src.join(file_name);
        let dst_path = dst.join(file_name);

        if src_path.exists() {
            debug!("Copying {} to {}", src_path.display(), dst_path.display());
            fs::copy(&src_path, &dst_path).await?;
        }
    }

    // Copy settings file to its specific location
    let old_settings = src.join("settings.json");
    if old_settings.exists() {
        let new_settings = fig_util::directories::settings_path()?;
        if !new_settings.exists() {
            if let Some(parent) = new_settings.parent() {
                fs::create_dir_all(parent).await?;
            }
            debug!("Copying settings to {}", new_settings.display());
            fs::copy(&old_settings, &new_settings).await?;
        }
    }

    // Copy global files from home directory
    copy_global_config_files()?;

    Ok(())
}

fn copy_global_config_files() -> Result<()> {
    let home_dir = fig_util::directories::home_dir()?;
    let kiro_dir = home_dir.join(".kiro");
    let legacy_amazonq_dir = home_dir.join(".aws/amazonq");

    let src_dir = if legacy_amazonq_dir.exists() {
        &legacy_amazonq_dir
    } else {
        return Ok(());
    };

    // Use fig_data_dir for knowledge_bases and cli-checkouts
    let data_dir = fig_util::directories::fig_data_dir()?;

    let files_to_copy = [
        // copy to home/.kiro
        ("cli-agents", kiro_dir.join("agents")),
        ("prompts", kiro_dir.join("prompts")),
        (".cli_bash_history", kiro_dir.join(".cli_bash_history")),
        // copy to data-dir
        ("cli-checkouts", data_dir.join("cli-checkouts")),
        ("knowledge_bases", data_dir.join("knowledge_bases")),
    ];

    for (src_subpath, dst_path) in files_to_copy {
        let src_path = src_dir.join(src_subpath);

        if src_path.exists() {
            if let Some(parent) = dst_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            if src_path.is_dir() {
                debug!("Copying directory {} to {}", src_path.display(), dst_path.display());
                dircpy::copy_dir(&src_path, &dst_path)?;
            } else {
                debug!("Copying file {} to {}", src_path.display(), dst_path.display());
                std::fs::copy(&src_path, &dst_path)?;
            }
        }
    }

    // Handle mcp.json separately with merging
    let src_mcp = src_dir.join("mcp.json");
    let dst_mcp = kiro_dir.join("settings/mcp.json");
    if src_mcp.exists() {
        merge_mcp_json(&src_mcp, &dst_mcp)?;
    }

    Ok(())
}

fn merge_mcp_json(src_path: &std::path::Path, dst_path: &std::path::Path) -> Result<()> {
    if let Some(parent) = dst_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let src_content = std::fs::read_to_string(src_path)?;
    let src_json = match serde_json::from_str::<Value>(&src_content) {
        Ok(json) => json,
        Err(_) => {
            debug!("Invalid JSON in source mcp.json, leaving destination unchanged");
            return Ok(());
        },
    };

    let merged_json = if dst_path.exists() {
        let dst_content = std::fs::read_to_string(dst_path)?;
        match serde_json::from_str::<Value>(&dst_content) {
            Ok(mut dst_json) => {
                if let (Some(src_servers), Some(dst_servers)) = (
                    src_json.get("mcpServers").and_then(|v| v.as_object()),
                    dst_json.get_mut("mcpServers").and_then(|v| v.as_object_mut()),
                ) {
                    for (key, value) in src_servers {
                        if !dst_servers.contains_key(key) {
                            dst_servers.insert(key.clone(), value.clone());
                        }
                    }
                }
                dst_json
            },
            Err(_) => {
                debug!("Invalid JSON in destination mcp.json, overwriting with source");
                src_json
            },
        }
    } else {
        src_json
    };

    let merged_content = serde_json::to_string_pretty(&merged_json)?;
    std::fs::write(dst_path, merged_content)?;
    debug!("Merged mcp.json to {}", dst_path.display());
    Ok(())
}

async fn migrate_dotfiles() -> Vec<(Shell, fig_integrations::Error)> {
    let shells = Shell::all();
    let mut errors = Vec::new();

    // First, collect all available shell integrations
    let mut shell_integrations = Vec::new();
    for shell in shells {
        match shell.get_shell_integrations(&Context::new()) {
            Ok(integrations) => {
                for integ in integrations {
                    shell_integrations.push((*shell, integ));
                }
            },
            Err(err) => errors.push((*shell, err)),
        }
    }

    // Because the fish shell doesn't support detecting legacy installations (and
    // therefore migrating), we're going to iterate through all shells (ie, bash and zsh) to see if a
    // legacy installation is detected. If so, then perform the migration.
    let mut has_legacy_installation = None;
    for (shell, integ) in &shell_integrations {
        if let Err(fig_integrations::Error::LegacyInstallation(_)) = integ.is_installed().await {
            has_legacy_installation = Some(shell);
        }
    }

    if let Some(shell) = has_legacy_installation {
        debug!(
            ?shell,
            "detected legacy dotfile installation, installing dotfile integrations"
        );
    } else {
        debug!("no legacy dotfile installation detected, not migrating dotfiles");
        return vec![];
    }

    for (shell, integration) in shell_integrations {
        if let Err(err) = integration.install().await {
            errors.push((shell, err));
        }
    }

    errors
}

fn mark_migration_completed() -> Result<()> {
    let db = fig_settings::sqlite::database()?;
    db.set_state_value(KIRO_MIGRATION_KEY, true)?;
    Ok(())
}

struct MigrationLock {
    _file: std::fs::File,
    path: PathBuf,
}

impl Drop for MigrationLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn acquire_migration_lock() -> Result<Option<MigrationLock>> {
    let lock_path = migration_lock_path()?;

    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Check if lock file exists and is stale
    if lock_path.exists() {
        match std::fs::metadata(&lock_path)
            .and_then(|m| m.modified())
            .and_then(|t| t.elapsed().map_err(std::io::Error::other))
        {
            Ok(elapsed) if elapsed.as_secs() > MIGRATION_TIMEOUT_SECS => {
                let _ = std::fs::remove_file(&lock_path);
            },
            _ => {
                // Continue with lock attempt even if metadata operations fail
                debug!(
                    "Failed to get lock file metadata, continuing with lock attempt: {}",
                    lock_path.display()
                );
            },
        }
    }

    // Try to acquire the lock
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;

    match flock(&file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(Some(MigrationLock {
            _file: file,
            path: lock_path,
        })),
        Err(_) => Ok(None),
    }
}

fn migration_lock_path() -> Result<PathBuf> {
    Ok(fig_util::directories::fig_data_dir()?.join("migration.lock"))
}

fn is_migration_completed() -> Result<bool> {
    Ok(fig_settings::state::get_bool_or(KIRO_MIGRATION_KEY, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "not in ci"]
    async fn integ_test_migration() {
        let _ = tracing_subscriber::fmt::try_init();
        migrate_if_needed().await.unwrap();
    }
}
