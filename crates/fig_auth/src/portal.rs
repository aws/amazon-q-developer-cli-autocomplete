// crates/fig_auth/src/portal.rs
// Build portal URL, listen for callback, and fan out to Social (exchange) or Internal (return PKCE
// params).

use std::collections::HashMap;
use std::env;
use std::time::Duration;

use bytes::Bytes;
use fig_util::system_info::is_mwinit_available;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::Service;
use hyper::{
    Request,
    Response,
};
use hyper_util::rt::TokioIo;
use rand::Rng;
use tokio::net::TcpListener;
use tracing::{
    debug,
    error,
    info,
    warn,
};

use crate::pkce::{
    generate_code_challenge,
    generate_code_verifier,
};
use crate::secret_store::SecretStore;
use crate::social::{
    CALLBACK_PORTS,
    SocialProvider,
    exchange_social_token,
};
use crate::{
    Error,
    Result,
};

const DEFAULT_AUTH_PORTAL_URL: &str = "https://app.kiro.dev";
const DEFAULT_AUTHORIZATION_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone)]
struct AuthPortalCallback {
    login_option: String,
    code: Option<String>,
    issuer_url: Option<String>,
    sso_region: Option<String>,
    state: String,
    path: String,
    error: Option<String>,
    error_description: Option<String>,
}

pub enum PortalResult {
    Social(SocialProvider),
    BuilderId {
        issuer_url: String,
        idc_region: String,
    },
    AwsIdc {
        issuer_url: String,
        idc_region: String,
    },
    /// Internal amazon user
    Internal {
        issuer_url: String,
        idc_region: String,
    },
}

/// Handle returned by init; caller opens the URL, then calls `finish_unified_portal`.
pub struct PortalInit {
    pub auth_url: String,
    pub state: String,
    pub verifier: String,
    pub listener: TcpListener,
    pub port: u16,
}

/// Step 1: prepare URL + bind a loopback callback port from the allowlist.
pub async fn init_unified_portal() -> Result<PortalInit> {
    // PKCE params for the portal & social exchange
    let verifier = generate_code_verifier();
    let challenge = generate_code_challenge(&verifier);

    let state: String = rand::rng()
        .sample_iter(rand::distr::Alphanumeric)
        .take(10)
        .map(char::from)
        .collect();

    // Bind one of the pre-allowed ports (must match Cognito/IdP allowlist)
    let listener = bind_allowed_port(CALLBACK_PORTS).await?;
    let port = listener.local_addr()?.port();
    let redirect_base = format!("http://localhost:{port}");
    info!(%port, %redirect_base, "Unified auth portal listening for callback");

    let auth_url = build_auth_url(&redirect_base, &state, &challenge);

    Ok(PortalInit {
        auth_url,
        state,
        verifier,
        listener,
        port,
    })
}

/// Step 2: wait for a single callback, then either exchange social tokens or return PKCE params.
pub async fn finish_unified_portal(init: PortalInit, secret_store: &SecretStore) -> Result<PortalResult> {
    let callback = wait_for_auth_callback(init.listener, init.state).await?;

    if let Some(error) = &callback.error {
        let friendly_msg =
            format_user_friendly_error(error, callback.error_description.as_deref(), &callback.login_option);
        warn!("OAuth error: {} - {}", error, friendly_msg);
        return Err(Error::OAuthCustomError(friendly_msg));
    }

    match callback.login_option.as_str() {
        "google" | "github" => {
            // Social: exchange tokens via shared auth service and persist them
            let provider = if callback.login_option == "google" {
                SocialProvider::Google
            } else {
                SocialProvider::Github
            };

            let code = match callback.code {
                Some(c) => c,
                None => return Err(Error::OAuthCustomError("Missing authorization code".to_string())),
            };

            let redirect_uri = format!(
                "http://localhost:{port}{path}?login_option={opt}",
                port = init.port,
                path = callback.path,
                opt = urlencoding::encode(&callback.login_option),
            );

            exchange_social_token(secret_store, provider, &init.verifier, &code, &redirect_uri).await?;
            Ok(PortalResult::Social(provider))
        },
        "internal" => {
            let (issuer_url, sso_region) = extract_sso_params(&callback, "internal")?;
            Ok(PortalResult::Internal {
                issuer_url,
                idc_region: sso_region,
            })
        },
        "awsidc" => {
            let (issuer_url, sso_region) = extract_sso_params(&callback, "awsIdc")?;
            Ok(PortalResult::AwsIdc {
                issuer_url,
                idc_region: sso_region,
            })
        },
        "builderid" => {
            let (issuer_url, sso_region) = extract_sso_params(&callback, "builderId")?;
            Ok(PortalResult::BuilderId {
                issuer_url,
                idc_region: sso_region,
            })
        },
        other => Err(Error::OAuthCustomError(format!("Unknown login_option: {other}"))),
    }
}

fn format_user_friendly_error(error_code: &str, description: Option<&str>, provider: &str) -> String {
    let cleaned_description = description.map(|d| {
        let first_part = d.split(';').next().unwrap_or(d);
        // Replace + with spaces (URL encoding)
        first_part.replace('+', " ").trim().to_string()
    });

    match error_code {
        "access_denied" => {
            format!(
                "{} denied access to Kiro. Please ensure you grant all required permissions.",
                provider
            )
        },
        "invalid_request" => "Authentication failed due to an invalid request. Please try again.".to_string(),
        "unauthorized_client" => "The application is not authorized. Please contact support.".to_string(),
        "server_error" => {
            format!("{} login is temporarily unavailable. Please try again later.", provider)
        },
        "invalid_scope" => "The requested permissions are invalid. Please contact support.".to_string(),
        _ => {
            // For unknown errors, use cleaned description or a generic message
            cleaned_description.unwrap_or_else(|| format!("Authentication failed: {}. Please try again.", error_code))
        },
    }
}

/// Build the authorization URL with all required parameters
fn build_auth_url(redirect_base: &str, state: &str, challenge: &str) -> String {
    let is_internal = is_mwinit_available();
    let internal_param = if is_internal { "&from_amazon_internal=true" } else { "" };
    let auth_portal_url = get_auth_portal_url();

    format!(
        "{}/signin?state={}&code_challenge={}&code_challenge_method=S256&redirect_uri={}{}&redirect_from=kirocli",
        auth_portal_url,
        state,
        challenge,
        urlencoding::encode(redirect_base),
        internal_param
    )
}

/// Extract issuer_url and sso_region from callback
fn extract_sso_params(callback: &AuthPortalCallback, auth_type: &str) -> Result<(String, String)> {
    let issuer_url = callback
        .issuer_url
        .clone()
        .ok_or_else(|| Error::OAuthCustomError(format!("Missing issuer_url for {} auth", auth_type)))?;

    let idc_region = callback
        .sso_region
        .clone()
        .ok_or_else(|| Error::OAuthCustomError(format!("Missing idc_region for {} auth", auth_type)))?;

    Ok((issuer_url, idc_region))
}

async fn wait_for_auth_callback(listener: TcpListener, expected_state: String) -> Result<AuthPortalCallback> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<AuthPortalCallback>(1);

    let server_handle = tokio::spawn(async move {
        const MAX_CONNECTIONS: usize = 3;
        let mut count = 0;

        loop {
            if count >= MAX_CONNECTIONS {
                warn!("Reached max connections ({})", MAX_CONNECTIONS);
                break;
            }

            match listener.accept().await {
                Ok((stream, _)) => {
                    count += 1;
                    debug!("Connection {}/{}", count, MAX_CONNECTIONS);

                    let io = TokioIo::new(stream);
                    let service = AuthCallbackService { tx: tx.clone() };

                    tokio::spawn(async move {
                        let _ = http1::Builder::new().serve_connection(io, service).await;
                    });
                },
                Err(e) => {
                    error!("Accept failed: {}", e);
                    break;
                },
            }
        }
    });

    let result = tokio::select! {
        v = rx.recv() => v.ok_or_else(|| Error::OAuthCustomError("Failed to receive callback".to_string())),
        _ = tokio::time::sleep(DEFAULT_AUTHORIZATION_TIMEOUT) => Err(Error::OAuthCustomError("Auth portal timed out".to_string())),
    }?;

    server_handle.abort();

    if result.state != expected_state {
        return Err(Error::OAuthCustomError(format!(
            "OAuth state mismatch: expected={expected_state}, actual={}",
            result.state
        )));
    }

    Ok(result)
}

#[derive(Clone)]
struct AuthCallbackService {
    tx: tokio::sync::mpsc::Sender<AuthPortalCallback>,
}

impl Service<Request<Incoming>> for AuthCallbackService {
    type Error = Error;
    type Future = std::pin::Pin<Box<dyn std::future::Future<Output = Result<Self::Response>> + Send>>;
    type Response = Response<Full<Bytes>>;

    fn call(&self, req: Request<Incoming>) -> Self::Future {
        let tx = self.tx.clone();
        Box::pin(async move {
            let uri = req.uri();
            let path = uri.path();

            if path == "/oauth/callback" || path == "/signin/callback" {
                handle_valid_callback(uri, path, tx).await
            } else {
                handle_invalid_callback(path).await
            }
        })
    }
}

/// Handle valid callback paths
async fn handle_valid_callback(
    uri: &hyper::Uri,
    path: &str,
    tx: tokio::sync::mpsc::Sender<AuthPortalCallback>,
) -> Result<Response<Full<Bytes>>> {
    let params: HashMap<String, String> = uri
        .query()
        .map(|q| {
            q.split('&')
                .filter_map(|kv| kv.split_once('='))
                .map(|(k, v)| (k.to_string(), urlencoding::decode(v).unwrap_or_default().to_string()))
                .collect()
        })
        .unwrap_or_default();

    let callback = AuthPortalCallback {
        login_option: params.get("login_option").cloned().unwrap_or_default(),
        code: params.get("code").cloned(),
        issuer_url: params.get("issuer_url").cloned(),
        sso_region: params.get("idc_region").cloned(),
        state: params.get("state").cloned().unwrap_or_default(),
        path: path.to_string(),
        error: params.get("error").cloned(),
        error_description: params.get("error_description").cloned(),
    };

    let _ = tx.send(callback.clone()).await;
    let auth_portal_url = get_auth_portal_url();

    // Determine redirect status based on error presence
    let redirect_url = if callback.error.is_some() {
        let error_msg = callback
            .error_description
            .as_deref()
            .unwrap_or(callback.error.as_deref().unwrap_or("Authentication failed"));
        format!(
            "{}/signin?auth_status=error&redirect_from=kirocli&error_message={}",
            auth_portal_url,
            urlencoding::encode(error_msg)
        )
    } else {
        format!("{}/signin?auth_status=success&redirect_from=kirocli", auth_portal_url)
    };

    Response::builder()
        .status(302)
        .header("Location", redirect_url)
        .header("Cache-Control", "no-store")
        .body(Full::new(Bytes::from("")))
        .map_err(|e| Error::OAuthCustomError(e.to_string()))
}

async fn handle_invalid_callback(path: &str) -> Result<Response<Full<Bytes>>> {
    debug!("Invalid callback path: {}", path);
    let auth_portal_url = get_auth_portal_url();

    let redirect_url = format!(
        "{}/signin?auth_status=error&redirect_from=kirocli&error_message={}",
        auth_portal_url,
        urlencoding::encode("Invalid callback path")
    );

    Response::builder()
        .status(302)
        .header("Location", redirect_url)
        .header("Cache-Control", "no-store")
        .body(Full::new(Bytes::from("")))
        .map_err(|e| Error::OAuthCustomError(e.to_string()))
}

async fn bind_allowed_port(ports: &[u16]) -> Result<TcpListener> {
    for port in ports {
        match TcpListener::bind(("127.0.0.1", *port)).await {
            Ok(l) => return Ok(l),
            Err(e) => debug!("Failed to bind to port {port}: {e}"),
        }
    }
    Err(Error::OAuthCustomError(
        "All callback ports are in use. Please close some applications and try again.".to_string(),
    ))
}

fn get_auth_portal_url() -> String {
    env::var("KIRO_AUTH_PORTAL_URL").unwrap_or_else(|_| DEFAULT_AUTH_PORTAL_URL.to_string())
}
