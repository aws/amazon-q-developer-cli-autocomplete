use serde::{
    Deserialize,
    Serialize,
};
use strum::{
    Display,
    EnumString,
};

/// The start URL for public builder ID users
pub const START_URL: &str = "https://view.awsapps.com/start";

#[derive(Debug, Copy, Clone, PartialEq, Eq, EnumString, Display, Serialize, Deserialize)]
pub enum OAuthFlow {
    DeviceCode,
    #[serde(alias = "Pkce")]
    PKCE,
}

#[derive(Debug, Clone, PartialEq, Eq, EnumString, Display, Serialize, Deserialize)]
pub enum TokenType {
    BuilderId,
    #[strum(serialize = "IdentityCenter")]
    IamIdentityCenter,
}

impl TokenType {
    pub fn from_start_url(start_url: Option<&str>) -> Self {
        match start_url {
            Some(url) if url == START_URL => TokenType::BuilderId,
            None => TokenType::BuilderId,
            Some(_) => TokenType::IamIdentityCenter,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    macro_rules! test_ser_deser {
        ($ty:ident, $variant:expr, $text:expr) => {
            let quoted = format!("\"{}\"", $text);
            assert_eq!(quoted, serde_json::to_string(&$variant).unwrap());
            assert_eq!($variant, serde_json::from_str(&quoted).unwrap());
            assert_eq!($variant, $ty::from_str($text).unwrap());
            assert_eq!($text, format!("{}", $variant));
        };
    }

    #[test]
    fn test_oauth_flow_ser_deser() {
        test_ser_deser!(OAuthFlow, OAuthFlow::DeviceCode, "DeviceCode");
        test_ser_deser!(OAuthFlow, OAuthFlow::PKCE, "PKCE");
        assert_eq!(OAuthFlow::PKCE, serde_json::from_str("\"Pkce\"").unwrap());
    }

    #[test]
    fn test_token_type_ser_deser() {
        test_ser_deser!(TokenType, TokenType::BuilderId, "BuilderId");
        test_ser_deser!(TokenType, TokenType::IamIdentityCenter, "IdentityCenter");
    }
}
