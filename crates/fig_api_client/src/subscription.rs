use amzn_codewhisperer_client::types::{
    OverageStatus,
    ResourceType,
    SubscriptionType,
};
use fig_auth::builder_id::TokenType;
use fig_auth::builder_id_token;
use serde::{
    Deserialize,
    Serialize,
};

use crate::{
    Client,
    Error,
};

#[derive(Debug)]
pub struct SubscriptionStatusInfo {
    pub tier: SubscriptionTier,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SubscriptionTier {
    Free,
    Pro,
    ProPlus,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UsageLimitsInfo {
    pub current_usage: Option<i32>,
    pub usage_limit: Option<i32>,
    pub overage_charges: Option<f64>,
    pub overage_enabled: bool,
    pub subscription_tier: SubscriptionTier,
    pub reset_date_utc: Option<String>,
}

const PRICING_DOCS_LINK: &str = "https://docs.aws.amazon.com/console/amazonq/subscriptions";

pub async fn generate_console_url() -> Result<String, Error> {
    let token = builder_id_token().await.ok().flatten();
    let region = token.as_ref().and_then(|t| t.region.clone());

    // IAM Identity Center (IdC) users's subscription is managed by admin
    if token
        .as_ref()
        .is_some_and(|t| matches!(t.token_type(), TokenType::IamIdentityCenter))
    {
        return Ok(PRICING_DOCS_LINK.to_string());
    }

    // Builder ID users
    let client = Client::new().await?;
    match client.create_subscription_token().await {
        Ok(r) => Ok(r
            .encoded_verification_url()
            .map_or_else(|| console_url(region.as_deref()), str::to_owned)),
        Err(e) => {
            if e.to_string().contains("ConflictException") {
                Ok(console_url(region.as_deref()))
            } else {
                Err(e)
            }
        },
    }
}

pub async fn get_usage_limits() -> Result<UsageLimitsInfo, Error> {
    let client = Client::new().await?;
    let response = client.get_usage_limits(Some(ResourceType::AgenticRequest)).await?;

    let subscription_tier = match response.subscription_info() {
        Some(info) => match info.r#type() {
            SubscriptionType::QDeveloperStandaloneFree => SubscriptionTier::Free,
            SubscriptionType::QDeveloperStandalone => SubscriptionTier::Pro,
            SubscriptionType::QDeveloperStandaloneProPlus => SubscriptionTier::ProPlus,
            _ => SubscriptionTier::Free,
        },
        None => SubscriptionTier::Free,
    };

    // Get usage breakdown
    let list = response.usage_breakdown_list();
    let ub = list
        .iter()
        .find(|b: &&amzn_codewhisperer_client::types::UsageBreakdown| {
            matches!(b.resource_type(), Some(ResourceType::AgenticRequest))
        })
        .unwrap_or_else(|| list.first().expect("usage_breakdown_list is not empty"));

    let current_usage = Some(ub.current_usage());
    let usage_limit = Some(ub.usage_limit());
    let overage_charges = Some(ub.overage_charges());

    let reset_date_utc = ub
        .next_date_reset()
        .and_then(|dt| std::time::SystemTime::try_from(*dt).ok())
        .map(|st| {
            let local: chrono::DateTime<chrono::Local> = st.into();
            local.format("%m/%d/%Y at %H:%M:%S").to_string()
        })
        .or_else(|| Some("1st of next month 12:00:00 GMT".into()));

    let overage_enabled = response
        .overage_configuration()
        .is_some_and(|c| matches!(c.overage_status(), OverageStatus::Enabled));

    Ok(UsageLimitsInfo {
        current_usage,
        usage_limit,
        overage_charges,
        overage_enabled,
        subscription_tier,
        reset_date_utc,
    })
}

fn console_url(region: Option<&str>) -> String {
    match region {
        Some(r) => format!("https://{r}.console.aws.amazon.com/amazonq/developer/home#/subscriptions"),
        None => "https://docs.aws.amazon.com/console/amazonq/upgrade-builder-id".to_string(),
    }
}
