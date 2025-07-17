use amzn_codewhisperer_client::types::UsageLimitList;
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
#[derive(Debug, Clone, Copy)]
pub enum SubscriptionTier {
    Free,
    Pro,
    Expiring,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UsageLimit {
    pub r#type: String,
    pub value: i64,
    pub percent_used: f64,
}

impl From<&UsageLimitList> for UsageLimit {
    fn from(limit: &UsageLimitList) -> Self {
        Self {
            r#type: format!("{:?}", limit.r#type()),
            value: limit.value(),
            percent_used: limit.percent_used().unwrap_or(0.0),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UsageLimitsInfo {
    pub limits: Vec<UsageLimit>,
    pub days_until_reset: i32,
}

pub async fn generate_console_url() -> Result<String, Error> {
    let token = builder_id_token().await;

    let region = match &token {
        Ok(Some(auth)) => auth.region.clone(),
        _ => None,
    };

    // IAM Identity Center (IdC) users always go to the console subscription page
    if let Ok(Some(auth)) = &token {
        if matches!(auth.token_type(), TokenType::IamIdentityCenter) {
            let region = region.unwrap_or_else(|| "us-east-1".to_string());
            return Ok(format!(
                "https://{}.console.aws.amazon.com/amazonq/developer/home#/subscriptions",
                region
            ));
        }
    }

    // Builder ID users
    let client = Client::new().await?;
    match client.create_subscription_token().await {
        Ok(response) => Ok(response.encoded_verification_url().to_string()),
        Err(e) => {
            let error_str = e.to_string();
            if error_str.contains("ConflictException") {
                if let Some(region) = region {
                    Ok(format!(
                        "https://{}.console.aws.amazon.com/amazonq/developer/home#/subscriptions",
                        region
                    ))
                } else {
                    Ok("https://docs.aws.amazon.com/console/amazonq/upgrade-builder-id".to_string())
                }
            } else {
                Err(e)
            }
        },
    }
}

pub async fn get_subscription_status() -> Result<SubscriptionStatusInfo, Error> {
    let token = builder_id_token().await;

    if let Ok(Some(auth)) = &token {
        if matches!(auth.token_type(), TokenType::IamIdentityCenter) {
            return Ok(SubscriptionStatusInfo {
                tier: SubscriptionTier::Pro,
            });
        }
    }

    // Default to Free for Builder ID users or if no token is available
    Ok(SubscriptionStatusInfo {
        tier: SubscriptionTier::Free,
    })
}

pub async fn get_usage_limits() -> Result<UsageLimitsInfo, Error> {
    let client = Client::new().await?;
    let response = client.get_usage_limits().await?;
    // specify type
    let limits: Vec<UsageLimit> = response.limits().iter().map(UsageLimit::from).collect();

    Ok(UsageLimitsInfo {
        limits,
        days_until_reset: response.days_until_reset(),
    })
}
