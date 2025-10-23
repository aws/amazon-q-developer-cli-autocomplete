use std::process::ExitCode;

use anstream::println;
use clap::{
    ArgGroup,
    Args,
    Parser,
    Subcommand,
    ValueEnum,
};
use eyre::{
    Result,
    WrapErr,
    bail,
};
use fig_auth::is_logged_in;
use fig_ipc::local::open_ui_element;
use fig_os_shim::Os;
use fig_proto::local::UiElement;
use fig_settings::JsonStore;
use fig_util::{
    CLI_BINARY_NAME,
    directories,
    manifest,
    system_info,
};
use globset::Glob;
use serde_json::json;

use super::OutputFormat;
use crate::cli::Cli;
use crate::util::desktop::{
    LaunchArgs,
    launch_fig_desktop,
};
use crate::util::{
    CliContext,
    app_not_running_message,
    qchat_path,
};

#[derive(Debug, Subcommand, PartialEq, Eq)]
pub enum SettingsSubcommands {
    /// Open the settings file
    Open,
    /// List settings
    List {
        /// Show all available settings
        #[arg(long)]
        all: bool,
        /// Format of the output
        #[arg(long, short, value_enum, default_value_t)]
        format: OutputFormat,
    },
    /// List configured settings
    #[command(hide = true)]
    All {
        /// Format of the output
        #[arg(long, short, value_enum, default_value_t)]
        format: OutputFormat,
    },
}

#[derive(Debug, Args, PartialEq, Eq)]
#[command(subcommand_negates_reqs = true)]
#[command(args_conflicts_with_subcommands = true)]
#[command(group(ArgGroup::new("vals").requires("key").args(&["value", "format"])))]
pub struct SettingsArgs {
    #[command(subcommand)]
    cmd: Option<SettingsSubcommands>,
    /// key
    key: Option<String>,
    /// value
    value: Option<String>,
    /// Delete a key (No value needed)
    #[arg(long, short)]
    delete: bool,
    /// Format of the output
    #[arg(long, short, value_enum, default_value_t)]
    format: OutputFormat,
}

impl SettingsArgs {
    pub async fn execute(&self, cli_context: &CliContext) -> Result<ExitCode> {
        macro_rules! print_connection_error {
            () => {
                println!("{}", app_not_running_message());
            };
        }

        match self.cmd {
            Some(SettingsSubcommands::Open) => {
                let file = directories::settings_path().context("Could not get settings path")?;
                if cli_context.context().platform().os() == Os::Mac {
                    tokio::process::Command::new("open").arg(file).output().await?;
                    Ok(ExitCode::SUCCESS)
                } else if let Ok(editor) = cli_context.context().env().get("EDITOR") {
                    tokio::process::Command::new(editor).arg(file).spawn()?.wait().await?;
                    Ok(ExitCode::SUCCESS)
                } else {
                    bail!("The EDITOR environment variable is not set")
                }
            },
            Some(SettingsSubcommands::List { all, format }) => {
                let mut args = vec!["settings".to_string(), "list".to_string()];
                if all {
                    args.push("--all".to_string());
                }
                if format != OutputFormat::default() {
                    args.push("--format".to_string());
                    args.push(format.to_possible_value().unwrap().get_name().to_string());
                }

                let status = tokio::process::Command::new(qchat_path()?).args(&args).status().await?;

                Ok(if status.success() {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                })
            },
            Some(SettingsSubcommands::All { format }) => {
                let mut args = vec!["settings".to_string(), "list".to_string()];

                if format != OutputFormat::default() {
                    args.push("--format".to_string());
                    args.push(format.to_possible_value().unwrap().get_name().to_string());
                }

                let status = tokio::process::Command::new(qchat_path()?).args(&args).status().await?;

                Ok(if status.success() {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                })
            },
            None => match &self.key {
                Some(key) => match (&self.value, self.delete) {
                    (Some(_), true) => Err(eyre::eyre!(
                        "the argument '--delete' cannot be used with '[VALUE]'\n Usage: q settings --delete {key}"
                    )),
                    (None, false) => match fig_settings::settings::get_value(key)? {
                        Some(value) => {
                            match self.format {
                                OutputFormat::Plain => match value.as_str() {
                                    Some(value) => println!("{value}"),
                                    None => println!("{value:#}"),
                                },
                                OutputFormat::Json => println!("{value}"),
                                OutputFormat::JsonPretty => println!("{value:#}"),
                            }
                            Ok(ExitCode::SUCCESS)
                        },
                        None => match self.format {
                            OutputFormat::Plain => Err(eyre::eyre!("No value associated with {key}")),
                            OutputFormat::Json | OutputFormat::JsonPretty => {
                                println!("null");
                                Ok(ExitCode::SUCCESS)
                            },
                        },
                    },
                    (Some(value_str), false) => {
                        let value = serde_json::from_str(value_str).unwrap_or_else(|_| json!(value_str));
                        fig_settings::settings::set_value(key, value)?;
                        Ok(ExitCode::SUCCESS)
                    },
                    (None, true) => {
                        let glob = Glob::new(key).context("Could not create glob")?.compile_matcher();
                        let settings = fig_settings::OldSettings::load()?;
                        let map = settings.map();
                        let keys_to_remove = map.keys().filter(|key| glob.is_match(key)).collect::<Vec<_>>();

                        match keys_to_remove.len() {
                            0 => {
                                return Err(eyre::eyre!("No settings found matching {key}"));
                            },
                            1 => {
                                println!("Removing {:?}", keys_to_remove[0]);
                                fig_settings::settings::remove_value(keys_to_remove[0])?;
                            },
                            _ => {
                                println!("Removing:");
                                for key in &keys_to_remove {
                                    println!("  - {key}");
                                }

                                for key in &keys_to_remove {
                                    fig_settings::settings::remove_value(key)?;
                                }
                            },
                        }

                        Ok(ExitCode::SUCCESS)
                    },
                },
                None => {
                    if self.delete {
                        return Err(eyre::eyre!(
                            "the argument '--delete' requires a <KEY>\n Usage: q settings --delete <KEY>"
                        ));
                    }

                    if manifest::is_minimal() || system_info::is_remote() {
                        Cli::parse_from([CLI_BINARY_NAME, "settings", "--help"]);
                        return Ok(ExitCode::SUCCESS);
                    }

                    launch_fig_desktop(LaunchArgs {
                        wait_for_socket: true,
                        open_dashboard: false,
                        immediate_update: true,
                        verbose: true,
                    })?;

                    if is_logged_in().await {
                        match open_ui_element(UiElement::Settings, None).await {
                            Ok(()) => Ok(ExitCode::SUCCESS),
                            Err(err) => {
                                print_connection_error!();
                                Err(err.into())
                            },
                        }
                    } else {
                        Ok(ExitCode::SUCCESS)
                    }
                },
            },
        }
    }
}
