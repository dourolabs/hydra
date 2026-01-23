use octocrab::Octocrab;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GithubAppClientIdResponse {
    pub client_id: String,
}

pub fn build_octocrab_client(token: &str) -> octocrab::Result<Octocrab> {
    Octocrab::builder()
        .personal_token(token.to_string())
        .build()
}
