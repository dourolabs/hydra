use serde::{Deserialize, Serialize};
use std::fmt;

fn default_imgur_api_base_url() -> String {
    "https://api.imgur.com".to_string()
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ImgurConfig {
    #[serde(alias = "IMGUR_CLIENT_ID")]
    pub client_id: String,
    #[serde(default, alias = "IMGUR_ACCESS_TOKEN")]
    pub access_token: Option<String>,
    #[serde(default = "default_imgur_api_base_url", alias = "IMGUR_API_BASE_URL")]
    pub api_base_url: String,
}

impl ImgurConfig {
    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn access_token(&self) -> Option<&str> {
        self.access_token.as_deref()
    }

    pub fn api_base_url(&self) -> &str {
        &self.api_base_url
    }
}

impl fmt::Debug for ImgurConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImgurConfig")
            .field("client_id", &self.client_id)
            .field(
                "access_token",
                &self.access_token.as_ref().map(|_| "<redacted>".to_string()),
            )
            .field("api_base_url", &self.api_base_url)
            .finish()
    }
}
