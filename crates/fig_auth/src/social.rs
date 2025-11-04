use std::fmt;

use aws_sdk_ssooidc::config::{
    ConfigBag,
    RuntimeComponents,
};
use aws_smithy_runtime_api::client::identity::http::Token;
use aws_smithy_runtime_api::client::identity::{
    Identity,
    IdentityFuture,
    ResolveIdentity,
};
use reqwest::Client;
use serde::{
    Deserialize,
    Serialize,
};
use serde_json::json;
use time::OffsetDateTime;
use tracing::{
    debug,
    error,
    trace,
};

use crate::consts::SOCIAL_AUTH_SERVICE_ENDPOINT;
use crate::secret_store::{
    Secret,
    SecretStore,
};
use crate::{
    Error,
    Result,
};

// NOTE: We use a fixed set of callback ports (not random) because:
// - IdP/Cognito only accepts pre-registered redirect URIs.
// - This list must match the Cognito allowlist
// - Bind only on loopback (127.0.0.1); never expose externally.
// - If all ports are in use, show a clear error.
// IMPORTANT: Do not change without auth service coordination.
pub const CALLBACK_PORTS: &[u16] = &[49153, 50153, 51153, 52153, 53153, 4649, 6588, 9091, 8008, 3128];
const USER_AGENT: &str = "Kiro-CLI";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
pub enum SocialProvider {
    #[serde(rename = "google")]
    #[value(name = "google")]
    Google,
    #[serde(rename = "github")]
    #[value(name = "github")]
    Github,
}

impl fmt::Display for SocialProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SocialProvider::Google => write!(f, "Google"),
            SocialProvider::Github => write!(f, "GitHub"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialToken {
    pub access_token: Secret,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    pub refresh_token: Option<Secret>,
    pub provider: SocialProvider,
    pub profile_arn: Option<String>,
}

impl SocialToken {
    pub const SECRET_KEY: &'static str = "codewhisperer:social:token";

    /// Load the token from the keychain, refresh the token if it is expired and return it
    pub async fn load(secret_store: &SecretStore, force_refresh: bool) -> Result<Option<Self>> {
        if cfg!(test) {
            return Ok(Some(Self {
                access_token: Secret("test_access_token".to_string()),
                expires_at: OffsetDateTime::now_utc() + time::Duration::minutes(60),
                refresh_token: Some(Secret("test_refresh_token".to_string())),
                provider: SocialProvider::Google,
                profile_arn: None,
            }));
        }

        trace!("loading social token from secret store");
        match secret_store.get(Self::SECRET_KEY).await {
            Ok(Some(secret)) => {
                let token: Option<Self> = serde_json::from_str(&secret.0)?;
                match token {
                    Some(mut token) => {
                        // if token is expired try to refresh
                        if token.is_expired() || force_refresh {
                            trace!("social token expired, refreshing");
                            token = token.refresh_token(secret_store).await?;
                        }
                        trace!(?token, "found a valid social token");
                        Ok(Some(token))
                    },
                    None => Ok(None),
                }
            },
            Ok(None) => Ok(None),
            Err(err) => {
                error!(%err, "Error getting social token");
                Err(err)?
            },
        }
    }

    pub async fn save(&self, secret_store: &SecretStore) -> Result<()> {
        secret_store
            .set(Self::SECRET_KEY, &serde_json::to_string(self)?)
            .await?;
        Ok(())
    }

    pub async fn save_profile_if_any(&self) -> Result<()> {
        if let Some(profile_arn) = &self.profile_arn {
            let profile_value = json!({
                "arn": profile_arn,
                "profile_name": "Social_Default_Profile"
            });

            if let Err(err) = fig_settings::state::set_value("api.codewhisperer.profile", profile_value) {
                error!(?err, profile_arn=%profile_arn, "failed to set profile from social token");
            }
        }
        Ok(())
    }

    pub async fn delete(&self, secret_store: &SecretStore) -> Result<()> {
        secret_store.delete(Self::SECRET_KEY).await?;
        Ok(())
    }

    pub fn is_expired(&self) -> bool {
        let now = OffsetDateTime::now_utc();
        (now + time::Duration::minutes(1)) > self.expires_at
    }

    pub async fn refresh_token(&self, secret_store: &SecretStore) -> Result<Self> {
        let Some(refresh_token) = &self.refresh_token else {
            error!("no refresh token found for social login");
            self.delete(secret_store).await.ok();
            return Err(Error::NoToken);
        };

        debug!("refreshing social access token");

        let client = Client::new();
        let response = client
            .post(format!("{}/refreshToken", SOCIAL_AUTH_SERVICE_ENDPOINT))
            .json(&serde_json::json!({ "refreshToken": refresh_token.0 }))
            .send()
            .await?;

        if response.status().is_success() {
            let token_response: TokenResponse = response.json().await?;
            let new_token = Self {
                access_token: Secret(token_response.access_token),
                expires_at: OffsetDateTime::now_utc() + time::Duration::seconds(token_response.expires_in as i64),
                refresh_token: Some(Secret(token_response.refresh_token)),
                provider: self.provider,
                profile_arn: token_response.profile_arn.clone().or(self.profile_arn.clone()),
            };

            new_token.save(secret_store).await?;
            Ok(new_token)
        } else {
            let status = response.status();
            error!("failed to refresh social token: {}", status);
            self.delete(secret_store).await.ok();
            Err(Error::OAuthCustomError(format!("Refresh failed: {}", status)))
        }
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(rename = "refreshToken")]
    refresh_token: String,
    #[serde(rename = "expiresIn")]
    expires_in: u64,
    #[serde(rename = "profileArn")]
    profile_arn: Option<String>,
}

/// Exchange authorization code for social tokens via the shared auth service and persist them.
/// - `store`: where to persist (SecretStore)
/// - `provider`: Google / GitHub (for bookkeeping)
/// - `code_verifier`: PKCE verifier used against the portal
/// - `code`: authorization code returned from the portal
/// - `redirect_uri`: exact redirect URI used by the local callback (must match the one sent to
///   portal)
pub async fn exchange_social_token(
    store: &SecretStore,
    provider: SocialProvider,
    code_verifier: &str,
    code: &str,
    redirect_uri: &str,
) -> Result<()> {
    let client = Client::new();
    let url = format!("{}/oauth/token", SOCIAL_AUTH_SERVICE_ENDPOINT);

    // Build JSON body that the shared auth service expects
    let req_body = serde_json::json!({
        "code": code,
        "code_verifier": code_verifier,
        "redirect_uri": redirect_uri,
    });

    let resp = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("User-Agent", USER_AGENT)
        .json(&req_body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_else(|_| "Unknown error".to_string());
        error!("token exchange failed: {} - {}", status, body);
        return Err(Error::OAuthCustomError(format!(
            "Token exchange failed: HTTP {} - {}",
            status, body
        )));
    }

    let tr: TokenResponse = resp.json().await?;

    // Persist using your existing `SocialToken` shape (Secret-wrapped fields)
    let token = SocialToken {
        access_token: Secret(tr.access_token),
        expires_at: OffsetDateTime::now_utc() + time::Duration::seconds(tr.expires_in as i64),
        refresh_token: Some(Secret(tr.refresh_token)),
        provider,
        profile_arn: tr.profile_arn,
    };

    token.save(store).await?;
    token.save_profile_if_any().await?;
    let _ = fig_settings::state::remove_value("api.selectedCustomization");
    Ok(())
}
#[derive(Debug, Clone)]
pub struct SocialBearerResolver;

impl ResolveIdentity for SocialBearerResolver {
    fn resolve_identity<'a>(
        &'a self,
        _runtime_components: &'a RuntimeComponents,
        _config_bag: &'a ConfigBag,
    ) -> IdentityFuture<'a> {
        IdentityFuture::new_boxed(Box::pin(async {
            let secret_store = SecretStore::new().await?;
            match SocialToken::load(&secret_store, false).await? {
                Some(token) => Ok(Identity::new(
                    Token::new(token.access_token.0.clone(), Some(token.expires_at.into())),
                    Some(token.expires_at.into()),
                )),
                None => Err(Error::NoToken.into()),
            }
        }))
    }
}

pub async fn is_social_logged_in(secret_store: &SecretStore) -> bool {
    matches!(SocialToken::load(secret_store, false).await, Ok(Some(_)))
}

pub async fn social_token(secret_store: &SecretStore) -> Result<Option<SocialToken>> {
    SocialToken::load(secret_store, false).await
}

pub async fn logout_social(secret_store: &SecretStore) -> Result<()> {
    secret_store.delete(SocialToken::SECRET_KEY).await?;
    Ok(())
}
