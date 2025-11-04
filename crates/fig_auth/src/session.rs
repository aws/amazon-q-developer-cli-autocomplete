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
use fig_settings::sqlite::database;
use tracing::{
    info,
    warn,
};

use crate::builder_id::{
    BuilderIdToken,
    DeviceRegistration,
    builder_id_token,
};
use crate::secret_store::SecretStore;
use crate::social::SocialToken;
use crate::{
    Error,
    Result,
};
pub async fn is_logged_in() -> bool {
    let builder_ok = match builder_id_token().await {
        Ok(Some(_)) => true,
        Ok(None) => {
            info!("not logged in with Builder ID - no valid token found");
            false
        },
        Err(err) => {
            warn!(?err, "failed to load a Builder ID token");
            false
        },
    };

    if builder_ok {
        return true;
    }

    let secret_store = match SecretStore::new().await {
        Ok(s) => s,
        Err(err) => {
            warn!(?err, "failed to open SecretStore for social login check");
            return false;
        },
    };

    match SocialToken::load(&secret_store, false).await {
        Ok(Some(_)) => true,
        Ok(None) => {
            info!("not logged in with Social - no valid token found");
            false
        },
        Err(err) => {
            warn!(?err, "failed to load a Social token");
            false
        },
    }
}

pub async fn logout() -> Result<()> {
    let Ok(secret_store) = SecretStore::new().await else {
        return Ok(());
    };

    let (builder_res, device_res, social_res) = tokio::join!(
        secret_store.delete(BuilderIdToken::SECRET_KEY),
        secret_store.delete(DeviceRegistration::SECRET_KEY),
        secret_store.delete(SocialToken::SECRET_KEY),
    );

    if let Ok(db) = database() {
        let _ = db.unset_auth_value(BuilderIdToken::SECRET_KEY);
        let _ = db.unset_auth_value(DeviceRegistration::SECRET_KEY);
        let _ = db.unset_auth_value(SocialToken::SECRET_KEY);
    }

    let profile_res = fig_settings::state::remove_value("api.codewhisperer.profile");

    builder_res?;
    device_res?;
    social_res?;
    profile_res?;

    Ok(())
}

/// Unified bearer token resolver that tries both social and builder ID tokens
#[derive(Debug, Clone)]
pub struct UnifiedBearerResolver;

impl ResolveIdentity for UnifiedBearerResolver {
    fn resolve_identity<'a>(
        &'a self,
        _runtime_components: &'a RuntimeComponents,
        _config_bag: &'a ConfigBag,
    ) -> IdentityFuture<'a> {
        IdentityFuture::new_boxed(Box::pin(async move {
            let secret_store = SecretStore::new().await?;

            if let Ok(Some(token)) = BuilderIdToken::load(&secret_store, false).await {
                return Ok(Identity::new(
                    Token::new(token.access_token.0.clone(), Some(token.expires_at.into())),
                    Some(token.expires_at.into()),
                ));
            }

            if let Ok(Some(token)) = SocialToken::load(&secret_store, false).await {
                return Ok(Identity::new(
                    Token::new(token.access_token.0.clone(), Some(token.expires_at.into())),
                    Some(token.expires_at.into()),
                ));
            }

            Err(Error::NoToken.into())
        }))
    }
}
