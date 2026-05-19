use std::path::Path;

use fig_util::consts::APP_BUNDLE_ID;
use fig_util::directories;
use tokio::fs;
use tokio::io::{
    AsyncReadExt,
    AsyncWriteExt,
};
use tracing::{
    error,
    warn,
};

use crate::Error;

async fn remove_in_dir_with_prefix_unless(dir: &Path, prefix: &str, unless: impl Fn(&str) -> bool) {
    if let Ok(mut entries) = fs::read_dir(dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with(prefix) && !unless(name) {
                    fs::remove_file(entry.path()).await.ok();
                    fs::remove_dir_all(entry.path()).await.ok();
                }
            }
        }
    }
}

#[allow(unused_variables)]
pub(crate) async fn uninstall_desktop(ctx: &fig_os_shim::Context) -> Result<(), Error> {
    // Remove launch agents
    if let Ok(home) = directories::home_dir() {
        let launch_agents = home.join("Library").join("LaunchAgents");
        remove_in_dir_with_prefix_unless(&launch_agents, "dev.precog.cli.", |p| p.contains("daemon")).await;
    } else {
        warn!("Could not find home directory");
    }

    tokio::process::Command::new("defaults")
        .args(["delete", APP_BUNDLE_ID])
        .output()
        .await
        .map_err(|err| warn!("Failed to delete defaults: {err}"))
        .ok();

    tokio::process::Command::new("defaults")
        .args(["delete", "dev.precog.cli.shared"])
        .output()
        .await
        .map_err(|err| warn!("Failed to delete defaults: {err}"))
        .ok();

    uninstall_terminal_integrations().await;

    if let Ok(fig_data_dir) = directories::fig_data_dir() {
        let state = fig_settings::state::get_string("anonymousId").unwrap_or_default();

        for file in std::fs::read_dir(fig_data_dir).ok().into_iter().flatten().flatten() {
            if let Some(file_name) = file.file_name().to_str() {
                if file_name == "credentials.json" {
                } else if file_name == "state.json" {
                    std::fs::write(file.path(), serde_json::json!({ "anonymousId": state }).to_string())
                        .map_err(|err| warn!("Failed to write state.json: {err}"))
                        .ok();
                } else if let Ok(metadata) = file.metadata() {
                    if metadata.is_dir() {
                        fs::remove_dir_all(file.path())
                            .await
                            .map_err(|err| warn!("Failed to remove data dir: {err}"))
                            .ok();
                    } else {
                        fs::remove_file(file.path())
                            .await
                            .map_err(|err| warn!("Failed to remove data dir: {err}"))
                            .ok();
                    }
                }
            }
        }
    }

    let app_path = fig_util::app_bundle_path();
    if app_path.exists() {
        fs::remove_dir_all(&app_path)
            .await
            .map_err(|err| warn!("Failed to remove {app_path:?}: {err}"))
            .ok();
    }

    if let Ok(old_fig_data_dir) = directories::old_fig_data_dir() {
        if old_fig_data_dir.exists() {
            if let Ok(metadata) = fs::symlink_metadata(&old_fig_data_dir).await {
                if metadata.is_symlink() {
                    fs::remove_file(&old_fig_data_dir)
                        .await
                        .map_err(|err| error!("Failed to remove the old fig data dir {old_fig_data_dir:?}: {err}"))
                        .ok();
                }
            }
        }
    }

    Ok(())
}

pub async fn uninstall_terminal_integrations() {
    if let Ok(home) = directories::home_dir() {
        for path in &[
            "Library/Application Support/iTerm2/Scripts/AutoLaunch/fig-iterm-integration.py",
            ".config/iterm2/AppSupport/Scripts/AutoLaunch/fig-iterm-integration.py",
            "Library/Application Support/iTerm2/Scripts/AutoLaunch/fig-iterm-integration.scpt",
        ] {
            fs::remove_file(home.join(path))
                .await
                .map_err(|err| warn!("Could not remove iTerm integration {path}: {err}"))
                .ok();
        }

        for (folder, prefix) in &[
            (".vscode/extensions", "withfig.fig-"),
            (".vscode-insiders/extensions", "withfig.fig-"),
            (".vscode-oss/extensions", "withfig.fig-"),
            (".cursor/extensions", "withfig.fig-"),
            (".cursor-nightly/extensions", "withfig.fig-"),
        ] {
            let folder = home.join(folder);
            remove_in_dir_with_prefix_unless(&folder, prefix, |_| false).await;
        }

        let hyper_path = home.join(".hyper.js");
        if hyper_path.exists() {
            match fs::File::open(&hyper_path).await {
                Ok(mut file) => {
                    let mut contents = String::new();
                    match file.read_to_string(&mut contents).await {
                        Ok(_) => {
                            contents = contents.replace("\"fig-hyper-integration\",", "");
                            contents = contents.replace("\"fig-hyper-integration\"", "");

                            match fs::File::create(&hyper_path).await {
                                Ok(mut file) => {
                                    file.write_all(contents.as_bytes())
                                        .await
                                        .map_err(|err| warn!("Could not write to Hyper config: {err}"))
                                        .ok();
                                },
                                Err(err) => {
                                    warn!("Could not create Hyper config: {err}");
                                },
                            }
                        },
                        Err(err) => {
                            warn!("Could not read Hyper config: {err}");
                        },
                    }
                },
                Err(err) => {
                    warn!("Could not open Hyper config: {err}");
                },
            }
        }

        let kitty_path = home.join(".config").join("kitty").join("kitty.conf");
        if kitty_path.exists() {
            match fs::File::open(&kitty_path).await {
                Ok(mut file) => {
                    let mut contents = String::new();
                    match file.read_to_string(&mut contents).await {
                        Ok(_) => {
                            contents = contents.replace("watcher ${HOME}/.fig/tools/kitty-integration.py", "");
                            match fs::File::create(&kitty_path).await {
                                Ok(mut file) => {
                                    file.write_all(contents.as_bytes())
                                        .await
                                        .map_err(|err| warn!("Could not write to Kitty config: {err}"))
                                        .ok();
                                },
                                Err(err) => {
                                    warn!("Could not create Kitty config: {err}");
                                },
                            }
                        },
                        Err(err) => {
                            warn!("Could not read Kitty config: {err}");
                        },
                    }
                },
                Err(err) => {
                    warn!("Could not open Kitty config: {err}");
                },
            }
        }
    }
}
