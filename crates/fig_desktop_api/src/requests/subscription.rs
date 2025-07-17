use fig_api_client::subscription::{
    SubscriptionTier as ApiTier,
    generate_console_url as generate_url,
    get_subscription_status as get_status,
    get_usage_limits as get_limits,
};
use fig_proto::fig::{
    GenerateConsoleUrlRequest,
    GenerateConsoleUrlResponse,
    GetSubscriptionStatusRequest,
    GetSubscriptionStatusResponse,
    GetUsageLimitsRequest,
    GetUsageLimitsResponse,
    SubscriptionTier,
    UsageLimit,
};
use tracing::debug;

use super::{
    RequestResult,
    RequestResultImpl,
    ServerOriginatedSubMessage,
};

pub async fn get_subscription_status(_request: GetSubscriptionStatusRequest) -> RequestResult {
    debug!("Getting subscription status");
    match get_status().await {
        Ok(status_info) => {
            let tier_proto = match status_info.tier {
                ApiTier::Free => SubscriptionTier::Free,
                ApiTier::Pro => SubscriptionTier::Pro,
                ApiTier::Expiring => SubscriptionTier::Expiring,
            } as i32;

            Ok(
                ServerOriginatedSubMessage::GetSubscriptionStatusResponse(GetSubscriptionStatusResponse {
                    tier: tier_proto,
                })
                .into(),
            )
        },
        Err(e) => RequestResult::error(format!("Failed to get subscription status: {e}")),
    }
}

pub async fn get_usage_limits(_request: GetUsageLimitsRequest) -> RequestResult {
    debug!("Getting usage limits");

    match get_limits().await {
        // yifan todo: type
        Ok(usage_info) => {
            let limits: Vec<UsageLimit> = usage_info
                .limits
                .into_iter()
                .filter(|l| l.r#type.eq_ignore_ascii_case("chat"))
                .map(|limit| UsageLimit {
                    r#type: limit.r#type,
                    value: limit.value,
                    percent_used: limit.percent_used,
                })
                .collect();

            Ok(
                ServerOriginatedSubMessage::GetUsageLimitsResponse(GetUsageLimitsResponse {
                    limits,
                    days_until_reset: usage_info.days_until_reset,
                })
                .into(),
            )
        },
        Err(e) => RequestResult::error(format!("Failed to get usage limits: {e}")),
    }
}

pub async fn generate_console_url(_request: GenerateConsoleUrlRequest) -> RequestResult {
    debug!("Generating console URL");

    match generate_url().await {
        Ok(url) => {
            Ok(ServerOriginatedSubMessage::GenerateConsoleUrlResponse(GenerateConsoleUrlResponse { url }).into())
        },
        Err(e) => RequestResult::error(format!("Failed to generate console URL: {e}")),
    }
}
