use std::fmt;
use std::fmt::Display;
use std::process::{
    ExitCode,
    exit,
};
use std::time::Duration;

use anstream::{
    eprintln,
    println,
};
use clap::{
    Args,
    Subcommand,
};
use crossterm::style::Stylize;
use dialoguer::Select;
use eyre::{
    Result,
    bail,
};
use serde_json::json;
use tokio::signal::ctrl_c;
use tracing::{
    error,
    info,
};

use super::OutputFormat;
use crate::api_client::list_available_profiles;
use crate::database::settings;
use serde_json;
use crate::auth::builder_id::{
    BuilderIdToken,
    PollCreateToken,
    TokenType,
    poll_create_token,
    start_device_authorization,
};
use crate::auth::pkce::start_pkce_authorization;
use crate::database::Database;
use crate::telemetry::{
    QProfileSwitchIntent,
    TelemetryResult,
    TelemetryThread,
};
use crate::util::spinner::{
    Spinner,
    SpinnerComponent,
};
use crate::util::system_info::is_remote;
use crate::util::{
    CLI_BINARY_NAME,
    PRODUCT_NAME,
    choose,
    input,
};

#[derive(Args, Debug, PartialEq, Eq, Clone, Default)]
pub struct LoginArgs {
    /// License type (pro for Identity Center, free for Builder ID)
    #[arg(long, value_enum)]
    pub license: Option<LicenseType>,

    /// Identity provider URL (for Identity Center)
    #[arg(long)]
    pub identity_provider: Option<String>,

    /// Region (for Identity Center)
    #[arg(long)]
    pub region: Option<String>,

    /// Always use the OAuth device flow for authentication. Useful for instances where browser
    /// redirects cannot be handled.
    #[arg(long)]
    pub use_device_flow: bool,
    
    /// Authentication strategy to use
    #[arg(long, value_enum)]
    pub auth_strategy: Option<super::shared::AuthStrategy>,
    
    /// AWS profile to use for SigV4 authentication
    #[arg(long)]
    pub aws_profile: Option<String>,
}

impl LoginArgs {
    pub async fn execute(self, database: &mut Database, telemetry: &TelemetryThread) -> Result<ExitCode> {
        if crate::auth::is_logged_in(database).await {
            eyre::bail!(
                "Already logged in, please logout with {} first",
                format!("{CLI_BINARY_NAME} logout").magenta()
            );
        }

        // If auth_strategy is explicitly set to SigV4, use that
        if let Some(super::shared::AuthStrategy::SigV4) = self.auth_strategy {
            // Save the auth strategy in the database settings
            let mut settings = settings::Settings::new().await?;
            settings.set(settings::Setting::AuthStrategy, "sigv4").await?;
            
            // If aws_profile is specified, save it
            // We can't directly set custom settings, so we'll skip this for now
            if let Some(profile) = &self.aws_profile {
                // In a real implementation, you'd save the profile somewhere
                println!("Using AWS profile: {}", profile);
            }
            settings.set(settings::Setting::AuthStrategy, "sigv4").await?;
            
            // If aws_profile is specified, save it
            if let Some(profile) = &self.aws_profile.clone() {
                // Use the new set_custom method to save the profile
                settings.set_custom("aws.profile", profile.as_str()).await?;
                println!("Using AWS profile: {}", profile);
            }
            
            // No actual login needed for SigV4 as it uses AWS credentials
            println!("Using SigV4 authentication with AWS credentials");
            telemetry.send_user_logged_in().ok();
            return Ok(ExitCode::SUCCESS);
        }

        let login_method = match self.license {
            Some(LicenseType::Free) => AuthMethod::BuilderId,
            Some(LicenseType::Pro) => AuthMethod::IdentityCenter,
            None => {
                if self.identity_provider.is_some() && self.region.is_some() {
                    // If license is specified and --identity-provider and --region are specified,
                    // the license is determined to be pro
                    AuthMethod::IdentityCenter
                } else {
                    // --license is not specified, prompt the user to choose
                    let options = [AuthMethod::BuilderId, AuthMethod::IdentityCenter, AuthMethod::SigV4];
                    let i = match choose("Select login method", &options)? {
                        Some(i) => i,
                        None => bail!("No login method selected"),
                    };
                    options[i]
                }
            },
        };

        match login_method {
            AuthMethod::SigV4 => {
                // Save the auth strategy in the database settings
                let mut settings = settings::Settings::new().await?;
                settings.set(settings::Setting::AuthStrategy, "sigv4").await?;
                
                // Prompt for AWS profile if not provided
                let profile = match self.aws_profile.clone() {
                    Some(p) => p,
                    None => input("Enter AWS profile name (optional)", None)?,
                };
                
                if !profile.is_empty() {
                    // Use the new set_custom method to save the profile
                    settings.set_custom("aws.profile", profile.as_str()).await?;
                    println!("Using AWS profile: {}", profile);
                }
                
                println!("Using SigV4 authentication with AWS credentials");
                telemetry.send_user_logged_in().ok();
            },
            AuthMethod::BuilderId | AuthMethod::IdentityCenter => {
                let (start_url, region) = match login_method {
                    AuthMethod::BuilderId => (None, None),
                    AuthMethod::IdentityCenter => {
                        let default_start_url = match self.identity_provider {
                            Some(start_url) => Some(start_url),
                            None => database.get_start_url()?,
                        };
                        let default_region = match self.region {
                            Some(region) => Some(region),
                            None => database.get_idc_region()?,
                        };

                        let start_url = input("Enter Start URL", default_start_url.as_deref())?;
                        let region = input("Enter Region", default_region.as_deref())?;

                        let _ = database.set_start_url(start_url.clone());
                        let _ = database.set_idc_region(region.clone());

                        (Some(start_url), Some(region))
                    },
                    AuthMethod::SigV4 => unreachable!(), // Already handled above
                };

                // Remote machine won't be able to handle browser opening and redirects,
                // hence always use device code flow.
                if is_remote() || self.use_device_flow {
                    try_device_authorization(database, telemetry, start_url.clone(), region.clone()).await?;
                } else {
                    let (client, registration) = start_pkce_authorization(start_url.clone(), region.clone()).await?;

                    match crate::util::open::open_url_async(&registration.url).await {
                        // If it succeeded, finish PKCE.
                        Ok(()) => {
                            let mut spinner = Spinner::new(vec![
                                SpinnerComponent::Spinner,
                                SpinnerComponent::Text(" Logging in...".into()),
                            ]);
                            let ctrl_c_stream = ctrl_c();
                            tokio::select! {
                                res = registration.finish(&client, Some(database)) => res?,
                                Ok(_) = ctrl_c_stream => {
                                    #[allow(clippy::exit)]
                                    exit(1);
                                },
                            }
                            telemetry.send_user_logged_in().ok();
                            spinner.stop_with_message("Logged in".into());
                        },
                        // If we are unable to open the link with the browser, then fallback to
                        // the device code flow.
                        Err(err) => {
                            error!(%err, "Failed to open URL with browser, falling back to device code flow");

                            // Try device code flow.
                            try_device_authorization(database, telemetry, start_url.clone(), region.clone()).await?;
                        },
                    }
                }
                
                // Save the auth strategy in the database
                if login_method == AuthMethod::BuilderId {
                    let mut settings = settings::Settings::new().await?;
                    settings.set(settings::Setting::AuthStrategy, "bearer").await?;
                }
            },
        };

        if login_method == AuthMethod::IdentityCenter {
            select_profile_interactive(database, telemetry, true).await?;
        }

        Ok(ExitCode::SUCCESS)
    }
}

pub async fn logout(database: &mut Database) -> Result<ExitCode> {
    let _ = crate::auth::logout(database).await;

    eprintln!("You are now logged out");
    eprintln!(
        "Run {} to log back in to {PRODUCT_NAME}",
        format!("{CLI_BINARY_NAME} login").magenta()
    );

    Ok(ExitCode::SUCCESS)
}

#[derive(Args, Debug, PartialEq, Eq, Clone, Default)]
pub struct WhoamiArgs {
    /// Output format to use
    #[arg(long, short, value_enum, default_value_t)]
    format: OutputFormat,
}

impl WhoamiArgs {
    pub async fn execute(self, database: &mut Database) -> Result<ExitCode> {
        // Check if using SigV4 auth
        let settings = settings::Settings::new().await?;
        if let Some(serde_json::Value::String(auth_strategy)) = settings.get(settings::Setting::AuthStrategy) {
            if auth_strategy == "sigv4" {
                // Get AWS profile if set
                let aws_profile = settings.get_custom("aws.profile")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                
                self.format.print(
                    || {
                        let profile_info = aws_profile
                            .as_ref()
                            .map(|p| format!(" with profile '{}'", p))
                            .unwrap_or_default();
                        format!("Using AWS credentials (SigV4) for authentication{}", profile_info)
                    },
                    || {
                        json!({
                            "accountType": "SigV4",
                            "awsProfile": aws_profile,
                        })
                    },
                );
                return Ok(ExitCode::SUCCESS);
            }
        }

        let builder_id = BuilderIdToken::load(database).await;

        match builder_id {
            Ok(Some(token)) => {
                self.format.print(
                    || match token.token_type() {
                        TokenType::BuilderId => "Logged in with Builder ID".into(),
                        TokenType::IamIdentityCenter => {
                            format!(
                                "Logged in with IAM Identity Center ({})",
                                token.start_url.as_ref().unwrap()
                            )
                        },
                    },
                    || {
                        json!({
                            "accountType": match token.token_type() {
                                TokenType::BuilderId => "BuilderId",
                                TokenType::IamIdentityCenter => "IamIdentityCenter",
                            },
                            "startUrl": token.start_url,
                            "region": token.region,
                        })
                    },
                );

                if matches!(token.token_type(), TokenType::IamIdentityCenter) {
                    if let Ok(Some(profile)) = database.get_auth_profile() {
                        color_print::cprintln!("\n<em>Profile:</em>\n{}\n{}\n", profile.profile_name, profile.arn);
                    }
                }

                Ok(ExitCode::SUCCESS)
            },
            _ => {
                self.format.print(|| "Not logged in", || json!({ "account": null }));
                Ok(ExitCode::FAILURE)
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum LicenseType {
    /// Free license with Builder ID
    Free,
    /// Pro license with Identity Center
    Pro,
}

pub async fn profile(database: &mut Database, telemetry: &TelemetryThread) -> Result<ExitCode> {
    if let Ok(Some(token)) = BuilderIdToken::load(database).await {
        if matches!(token.token_type(), TokenType::BuilderId) {
            bail!("This command is only available for Pro users");
        }
    }

    select_profile_interactive(database, telemetry, false).await?;

    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthMethod {
    /// Builder ID (free)
    BuilderId,
    /// IdC (enterprise)
    IdentityCenter,
    /// AWS SigV4
    SigV4,
}

impl Display for AuthMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthMethod::BuilderId => write!(f, "Use for Free with Builder ID"),
            AuthMethod::IdentityCenter => write!(f, "Use with Pro license"),
            AuthMethod::SigV4 => write!(f, "Use with AWS credentials (SigV4)"),
        }
    }
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum UserSubcommand {
    Profile,
}

async fn try_device_authorization(
    database: &mut Database,
    telemetry: &TelemetryThread,
    start_url: Option<String>,
    region: Option<String>,
) -> Result<()> {
    let device_auth = start_device_authorization(database, start_url.clone(), region.clone()).await?;

    println!();
    println!("Confirm the following code in the browser");
    println!("Code: {}", device_auth.user_code.bold());
    println!();

    let print_open_url = || println!("Open this URL: {}", device_auth.verification_uri_complete);

    if is_remote() {
        print_open_url();
    } else if let Err(err) = crate::util::open::open_url_async(&device_auth.verification_uri_complete).await {
        error!(%err, "Failed to open URL with browser");
        print_open_url();
    }

    let mut spinner = Spinner::new(vec![
        SpinnerComponent::Spinner,
        SpinnerComponent::Text(" Logging in...".into()),
    ]);

    loop {
        let ctrl_c_stream = ctrl_c();
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(device_auth.interval.try_into().unwrap_or(1))) => (),
            Ok(_) = ctrl_c_stream => {
                #[allow(clippy::exit)]
                exit(1);
            }
        }
        match poll_create_token(
            database,
            device_auth.device_code.clone(),
            start_url.clone(),
            region.clone(),
        )
        .await
        {
            PollCreateToken::Pending => {},
            PollCreateToken::Complete => {
                telemetry.send_user_logged_in().ok();
                spinner.stop_with_message("Logged in".into());
                break;
            },
            PollCreateToken::Error(err) => {
                spinner.stop();
                return Err(err.into());
            },
        };
    }
    Ok(())
}

async fn select_profile_interactive(database: &mut Database, telemetry: &TelemetryThread, whoami: bool) -> Result<()> {
    let mut spinner = Spinner::new(vec![
        SpinnerComponent::Spinner,
        SpinnerComponent::Text(" Fetching profiles...".into()),
    ]);
    let profiles = list_available_profiles(database).await?;
    if profiles.is_empty() {
        info!("Available profiles was empty");
        return Ok(());
    }

    let sso_region = database.get_idc_region()?;
    let total_profiles = profiles.len() as i64;

    if whoami && profiles.len() == 1 {
        if let Some(profile_region) = profiles[0].arn.split(':').nth(3) {
            telemetry
                .send_profile_state(
                    QProfileSwitchIntent::Update,
                    profile_region.to_string(),
                    TelemetryResult::Succeeded,
                    sso_region,
                )
                .ok();
        }

        spinner.stop_with_message(String::new());
        database.set_auth_profile(&profiles[0])?;
        return Ok(());
    }

    let mut items: Vec<String> = profiles
        .iter()
        .map(|p| format!("{} (arn: {})", p.profile_name, p.arn))
        .collect();
    let active_profile = database.get_auth_profile()?;

    if let Some(default_idx) = active_profile
        .as_ref()
        .and_then(|active| profiles.iter().position(|p| p.arn == active.arn))
    {
        items[default_idx] = format!("{} (active)", items[default_idx].as_str());
    }

    spinner.stop_with_message(String::new());
    let selected = Select::with_theme(&crate::util::dialoguer_theme())
        .with_prompt("Select an IAM Identity Center profile")
        .items(&items)
        .default(0)
        .interact_opt()?;

    match selected {
        Some(i) => {
            let chosen = &profiles[i];
            eprintln!("Profile set");
            database.set_auth_profile(chosen)?;

            if let Some(profile_region) = chosen.arn.split(':').nth(3) {
                let intent = if whoami {
                    QProfileSwitchIntent::Auth
                } else {
                    QProfileSwitchIntent::User
                };

                telemetry
                    .send_did_select_profile(
                        intent,
                        profile_region.to_string(),
                        TelemetryResult::Succeeded,
                        sso_region,
                        Some(total_profiles),
                    )
                    .ok();
            }
        },
        None => {
            telemetry
                .send_did_select_profile(
                    QProfileSwitchIntent::User,
                    "not-set".to_string(),
                    TelemetryResult::Cancelled,
                    sso_region,
                    Some(total_profiles),
                )
                .ok();

            bail!("No profile selected.\n");
        },
    }

    Ok(())
}
