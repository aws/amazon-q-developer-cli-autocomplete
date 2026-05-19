pub mod cognito;
mod dispatch;
pub mod endpoint;
mod event;
mod install_method;
mod util;

use std::any::Any;
use std::sync::LazyLock;
use std::time::Duration;

use amzn_toolkit_telemetry_client::config::{
    BehaviorVersion,
    Region,
};
use amzn_toolkit_telemetry_client::error::DisplayErrorContext;
use amzn_toolkit_telemetry_client::types::AwsProduct;
use amzn_toolkit_telemetry_client::{
    Client as ToolkitTelemetryClient,
    Config,
};
use aws_credential_types::provider::SharedCredentialsProvider;
use cognito::CognitoProvider;
use dispatch::dispatch;
pub use dispatch::{
    DispatchMode,
    dispatch_mode,
    set_dispatch_mode,
};
use endpoint::StaticEndpoint;
pub use event::{
    AppTelemetryEvent,
    InlineShellCompletionActionedOptions,
};
use fig_aws_common::app_name;
use fig_settings::State;
use fig_telemetry_core::{
    Event,
    QProfileSwitchIntent,
    TelemetryEmitter,
    TelemetryResult,
};
pub use fig_telemetry_core::{
    EventType,
    SuggestionState,
};
use fig_util::Shell;
use fig_util::system_info::os_version;
use fig_util::terminal::{
    current_terminal,
    current_terminal_version,
};
pub use install_method::{
    InstallMethod,
    get_install_method,
};
use tokio::sync::{
    Mutex,
    OnceCell,
};
use tokio::task::JoinSet;
use tracing::{
    debug,
    error,
};
use util::{
    old_client_id,
    telemetry_is_disabled,
};
use uuid::Uuid;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Telemetry is disabled")]
    TelemetryDisabled,
    #[error(transparent)]
    ClientError(#[from] amzn_toolkit_telemetry_client::operation::post_metrics::PostMetricsError),
}

async fn client() -> &'static Client {
    static CLIENT: OnceCell<Client> = OnceCell::const_new();
    CLIENT
        .get_or_init(|| async { Client::new(TelemetryStage::EXTERNAL_PROD).await })
        .await
}

/// A telemetry emitter that first tries sending the event to figterm so that the CLI commands can
/// execute much quicker. Only falls back to sending it directly on the current task if sending to
/// figterm fails.
struct DispatchingTelemetryEmitter;

#[async_trait::async_trait]
impl TelemetryEmitter for DispatchingTelemetryEmitter {
    async fn send(&self, event: fig_telemetry_core::Event) {
        let event = AppTelemetryEvent::from_event(event).await;
        dispatch_or_send_event(event).await;
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn init_global_telemetry_emitter() {
    fig_telemetry_core::init_global_telemetry_emitter(DispatchingTelemetryEmitter {});
}

/// A IDE toolkit telemetry stage
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TelemetryStage {
    pub name: &'static str,
    pub endpoint: &'static str,
    pub cognito_pool_id: &'static str,
    pub region: Region,
}

impl TelemetryStage {
    #[allow(dead_code)]
    const BETA: Self = Self::new(
        "beta",
        "https://7zftft3lj2.execute-api.us-east-1.amazonaws.com/Beta",
        "us-east-1:db7bfc9f-8ecd-4fbb-bea7-280c16069a99",
        "us-east-1",
    );
    const EXTERNAL_PROD: Self = Self::new(
        "prod",
        "https://client-telemetry.us-east-1.amazonaws.com",
        "us-east-1:820fd6d1-95c0-4ca4-bffb-3f01d32da842",
        "us-east-1",
    );

    const fn new(
        name: &'static str,
        endpoint: &'static str,
        cognito_pool_id: &'static str,
        region: &'static str,
    ) -> Self {
        Self {
            name,
            endpoint,
            cognito_pool_id,
            region: Region::from_static(region),
        }
    }
}

static JOIN_SET: LazyLock<Mutex<JoinSet<()>>> = LazyLock::new(|| Mutex::new(JoinSet::new()));

/// Joins all current telemetry events
pub async fn finish_telemetry() {
    let mut set = JOIN_SET.lock().await;
    while let Some(res) = set.join_next().await {
        if let Err(err) = res {
            error!(%err, "Failed to join telemetry event");
        }
    }
}

/// Joins all current telemetry events and panics if any fail to join
pub async fn finish_telemetry_unwrap() {
    let mut set = JOIN_SET.lock().await;
    while let Some(res) = set.join_next().await {
        res.unwrap();
    }
}

#[derive(Debug, Clone)]
pub struct Client {
    client_id: Uuid,
    toolkit_telemetry_client: Option<ToolkitTelemetryClient>,
    state: State,
}

impl Client {
    pub async fn new(telemetry_stage: TelemetryStage) -> Self {
        let client_id = util::get_client_id();
        let toolkit_telemetry_client = Some(amzn_toolkit_telemetry_client::Client::from_conf(
            Config::builder()
                .http_client(fig_aws_common::http_client::client())
                .behavior_version(BehaviorVersion::v2025_08_07())
                .endpoint_resolver(StaticEndpoint(telemetry_stage.endpoint))
                .app_name(app_name())
                .region(telemetry_stage.region.clone())
                .credentials_provider(SharedCredentialsProvider::new(CognitoProvider::new(telemetry_stage)))
                .build(),
        ));
        let state = State::new();

        Self {
            client_id,
            toolkit_telemetry_client,
            state,
        }
    }

    pub fn mock() -> Self {
        let client_id = util::get_client_id();
        let toolkit_telemetry_client = None;
        let state = State::new_fake();

        Self {
            client_id,
            toolkit_telemetry_client,
            state,
        }
    }

    async fn send_event(&self, event: AppTelemetryEvent) {
        self.send_migrate().await;
        self.send_telemetry_toolkit_metric(event).await;
    }

    async fn send_migrate(&self) {
        // If we have not sent the migrate event, send this event
        match self.state.atomic_bool_or("telemetry.sentMigrateClientIdEvent", true) {
            Ok(true) => {},
            Ok(false) => {
                if let Some(old_client_id) = old_client_id() {
                    let event = AppTelemetryEvent::from_event(Event::new(EventType::MigrateClientId {
                        old_client_id: old_client_id.into(),
                    }))
                    .await;
                    self.send_telemetry_toolkit_metric(event).await;
                }
            },
            Err(err) => error!(
                %err,
                "Failed to atomic_bool_or telemetry.sentMigrateClientIdEvent, skipping migrate event"
            ),
        }
    }

    async fn send_telemetry_toolkit_metric(&self, event: AppTelemetryEvent) {
        if telemetry_is_disabled() {
            return;
        }
        let Some(toolkit_telemetry_client) = self.toolkit_telemetry_client.clone() else {
            return;
        };
        let client_id = self.client_id;
        let Some(metric_datum) = event.into_metric_datum() else {
            return;
        };

        let mut set = JOIN_SET.lock().await;
        set.spawn({
            async move {
                let product = AwsProduct::CodewhispererTerminal;
                let product_version = env!("CARGO_PKG_VERSION");
                let os = std::env::consts::OS;
                let os_architecture = std::env::consts::ARCH;
                let os_version = os_version().map(|v| v.to_string()).unwrap_or_default();
                let metric_name = metric_datum.metric_name().to_owned();

                debug!(?product, ?metric_datum, "Posting metrics");
                if let Err(err) = toolkit_telemetry_client
                    .post_metrics()
                    .aws_product(product)
                    .aws_product_version(product_version)
                    .client_id(client_id)
                    .os(os)
                    .os_architecture(os_architecture)
                    .os_version(os_version)
                    .metric_data(metric_datum)
                    .send()
                    .await
                    .map_err(DisplayErrorContext)
                {
                    error!(%err, ?metric_name, "Failed to post metric");
                }
            }
        });
    }
}

pub async fn send_event(event: AppTelemetryEvent) {
    client().await.send_event(event).await;
}

pub async fn dispatch_or_send_event(event: AppTelemetryEvent) {
    debug!(?event, "Dispatching telemetry event");
    if dispatch(&event).await.should_fallback() {
        debug!(?event, "Dispatch failed, falling back to send_event");
        send_event(event).await;
    }
}

pub async fn send_user_logged_in() {
    let event = AppTelemetryEvent::new(EventType::UserLoggedIn {}).await;
    dispatch_or_send_event(event).await;
}

pub async fn send_completion_inserted(command: String, terminal: Option<String>, shell: Option<String>) {
    let event = AppTelemetryEvent::new(EventType::CompletionInserted {
        command,
        terminal,
        shell,
    })
    .await;
    dispatch_or_send_event(event).await;
}

pub async fn send_translation_actioned(latency: Duration, suggestion_state: SuggestionState) {
    let (shell, shell_version) = shell().await;
    let event = AppTelemetryEvent::new(EventType::TranslationActioned {
        latency,
        suggestion_state,
        terminal: current_terminal().map(|t| t.internal_id().to_string()),
        terminal_version: current_terminal_version().map(Into::into),
        shell: shell.map(|s| s.to_string()),
        shell_version,
    })
    .await;
    dispatch_or_send_event(event).await;
}

pub async fn send_cli_subcommand_executed(subcommand: impl Into<String>) {
    let (shell, shell_version) = shell().await;
    let event = AppTelemetryEvent::new(EventType::CliSubcommandExecuted {
        subcommand: subcommand.into(),
        terminal: current_terminal().map(|t| t.internal_id().to_string()),
        terminal_version: current_terminal_version().map(Into::into),
        shell: shell.map(|s| s.to_string()),
        shell_version,
    })
    .await;
    dispatch_or_send_event(event).await;
}

pub async fn send_doctor_check_failed(failed_check: impl Into<String>) {
    let (shell, shell_version) = shell().await;
    let event = AppTelemetryEvent::new(EventType::DoctorCheckFailed {
        doctor_check: failed_check.into(),
        terminal: current_terminal().map(|t| t.internal_id().to_string()),
        terminal_version: current_terminal_version().map(Into::into),
        shell: shell.map(|s| s.to_string()),
        shell_version,
    })
    .await;
    dispatch_or_send_event(event).await;
}

pub async fn send_dashboard_page_viewed(route: impl Into<String>) {
    let event = AppTelemetryEvent::new(EventType::DashboardPageViewed { route: route.into() }).await;
    dispatch_or_send_event(event).await;
}

pub async fn send_menu_bar_actioned(menu_bar_item: Option<impl Into<String>>) {
    let event = AppTelemetryEvent::new(EventType::MenuBarActioned {
        menu_bar_item: menu_bar_item.map(|i| i.into()),
    })
    .await;
    dispatch_or_send_event(event).await;
}

pub async fn send_fig_user_migrated() {
    let event = AppTelemetryEvent::new(EventType::FigUserMigrated {}).await;
    dispatch_or_send_event(event).await;
}

pub async fn send_start_chat(conversation_id: String) {
    let event = AppTelemetryEvent::new(EventType::ChatStart { conversation_id }).await;
    dispatch_or_send_event(event).await;
}

pub async fn send_end_chat(conversation_id: String) {
    let event = AppTelemetryEvent::new(EventType::ChatEnd { conversation_id }).await;
    dispatch_or_send_event(event).await;
}

pub async fn send_chat_added_message(conversation_id: String, message_id: String, context_file_length: Option<usize>) {
    let event = AppTelemetryEvent::new(EventType::ChatAddedMessage {
        conversation_id,
        message_id,
        context_file_length,
    })
    .await;
    dispatch_or_send_event(event).await;
}

async fn shell() -> (Option<Shell>, Option<String>) {
    Shell::current_shell_version()
        .await
        .map(|(shell, shell_version)| (Some(shell), Some(shell_version)))
        .unwrap_or((None, None))
}

pub async fn send_did_select_profile(
    source: QProfileSwitchIntent,
    amazonq_profile_region: String,
    result: TelemetryResult,
    sso_region: Option<String>,
    profile_count: Option<i64>,
) {
    let event = AppTelemetryEvent::new(EventType::DidSelectProfile {
        source,
        amazonq_profile_region,
        result,
        sso_region,
        profile_count,
    })
    .await;
    dispatch_or_send_event(event).await;
}

pub async fn send_profile_state(
    source: QProfileSwitchIntent,
    amazonq_profile_region: String,
    result: TelemetryResult,
    sso_region: Option<String>,
) {
    let event = AppTelemetryEvent::new(EventType::ProfileState {
        source,
        amazonq_profile_region,
        result,
        sso_region,
    })
    .await;
    dispatch_or_send_event(event).await;
}

#[cfg(test)]
mod test {
    use event::tests::all_events;
    use fig_util::CLI_BINARY_NAME;

    use super::*;

    #[tokio::test]
    async fn client_send_event_test() {
        let client = Client::mock();
        for event in all_events().await {
            client.send_event(event).await;
        }
    }

    #[tracing_test::traced_test]
    #[tokio::test]
    #[ignore = "needs network"]
    async fn test_all_telemetry() {
        send_user_logged_in().await;
        send_completion_inserted(CLI_BINARY_NAME.to_owned(), None, None).await;
        send_translation_actioned(Duration::from_millis(10), SuggestionState::Accept).await;
        send_cli_subcommand_executed("doctor").await;
        send_doctor_check_failed("").await;
        send_dashboard_page_viewed("/").await;
        send_menu_bar_actioned(Some("Settings")).await;
        send_chat_added_message("debug".to_owned(), "debug".to_owned(), Some(123)).await;

        finish_telemetry_unwrap().await;

        assert!(!logs_contain("ERROR"));
        assert!(!logs_contain("error"));
        assert!(!logs_contain("WARN"));
        assert!(!logs_contain("warn"));
        assert!(!logs_contain("Failed to post metric"));
    }
}
