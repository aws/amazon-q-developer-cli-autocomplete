use aws_config::Region;

// Endpoint constants
pub const PROD_CODEWHISPERER_ENDPOINT_URL: &str = "https://codewhisperer.us-east-1.amazonaws.com";
pub const PROD_CODEWHISPERER_ENDPOINT_REGION: Region = Region::from_static("us-east-1");

pub const PROD_Q_ENDPOINT_URL: &str = "https://q.us-east-1.amazonaws.com";
pub const PROD_Q_ENDPOINT_REGION: Region = Region::from_static("us-east-1");

// FRA endpoint constants
pub const PROD_CODEWHISPERER_FRA_ENDPOINT_URL: &str = "https://q.eu-central-1.amazonaws.com/";
pub const PROD_CODEWHISPERER_FRA_ENDPOINT_REGION: Region = Region::from_static("eu-central-1");

// Opt out constants
pub const X_AMZN_CODEWHISPERER_OPT_OUT_HEADER: &str = "x-amzn-codewhisperer-optout";

// Dummy value that simply provides subscription status (for the holder of a given bearer token)
// This is a design decision on the service-side to avoid creating a dedicated status API.
#[allow(dead_code)] // TODO: Remove
pub const SUBSCRIPTION_STATUS_ACCOUNT_ID: &str = "111111111111";
