use fig_api_client::subscription::{
    SubscriptionTier as ApiTier,
    generate_console_url as generate_url,
    get_usage_limits as get_limits,
};
use fig_proto::fig::{
    GenerateConsoleUrlRequest,
    GenerateConsoleUrlResponse,
    GetUsageLimitsRequest,
    GetUsageLimitsResponse,
    OverageInfo,
    SubscriptionInfo,
    SubscriptionTier,
    UsageBreakdown,
};
use tracing::debug;

use super::{
    RequestResult,
    RequestResultImpl,
    ServerOriginatedSubMessage,
};

pub async fn get_usage_limits(_request: GetUsageLimitsRequest) -> RequestResult {
    debug!("Getting usage limits");

    match get_limits().await {
        Ok(usage_info) => {
            let usage_breakdown = UsageBreakdown {
                current_usage: usage_info.current_usage.unwrap_or(0),
                usage_limit: usage_info.usage_limit.unwrap_or(0),
                overage_charges: usage_info.overage_charges.unwrap_or(0.0),
                reset_date: usage_info
                    .reset_date_utc
                    .unwrap_or_else(|| "1st of next month 12:00:00 GMT".to_string()),
            };

            let subscription_info = SubscriptionInfo {
                tier: match usage_info.subscription_tier {
                    ApiTier::Free => SubscriptionTier::Free,
                    ApiTier::Pro => SubscriptionTier::Pro,
                    ApiTier::ProPlus => SubscriptionTier::ProPlus,
                } as i32,
            };

            let overage_info = OverageInfo {
                enabled: usage_info.overage_enabled,
            };

            Ok(
                ServerOriginatedSubMessage::GetUsageLimitsResponse(GetUsageLimitsResponse {
                    usage_breakdown: Some(usage_breakdown),
                    subscription_info: Some(subscription_info),
                    overage_info: Some(overage_info),
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
