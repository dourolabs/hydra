use crate::{
    app::{AppState, ServiceState},
    store::MemoryStore,
};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::{MockJobEngine, TestStateHandles, test_app_config};

pub fn github_user_response(login: &str, id: u64) -> serde_json::Value {
    json!({
        "login": login,
        "id": id,
        "node_id": "NODEID",
        "avatar_url": "https://example.com/avatar",
        "gravatar_id": "gravatar",
        "url": "https://example.com/user",
        "html_url": "https://example.com/user",
        "followers_url": "https://example.com/followers",
        "following_url": "https://example.com/following",
        "gists_url": "https://example.com/gists",
        "starred_url": "https://example.com/starred",
        "subscriptions_url": "https://example.com/subscriptions",
        "organizations_url": "https://example.com/orgs",
        "repos_url": "https://example.com/repos",
        "events_url": "https://example.com/events",
        "received_events_url": "https://example.com/received_events",
        "type": "User",
        "site_admin": false,
        "name": null,
        "patch_url": null,
        "email": null
    })
}

pub fn test_state_with_github_api_base_url(api_base_url: String) -> TestStateHandles {
    test_state_with_github_urls(api_base_url, "https://github.com".to_string())
}

pub fn test_state_with_github_urls(
    api_base_url: String,
    oauth_base_url: String,
) -> TestStateHandles {
    test_state_with_github_urls_and_allowed_orgs(api_base_url, oauth_base_url, Vec::new())
}

pub fn test_state_with_github_urls_and_allowed_orgs(
    api_base_url: String,
    oauth_base_url: String,
    allowed_orgs: Vec<String>,
) -> TestStateHandles {
    let mut config = test_app_config();
    config.github_app.api_base_url = api_base_url;
    config.github_app.oauth_base_url = oauth_base_url;
    config.metis.allowed_orgs = allowed_orgs;

    let store = Arc::new(MemoryStore::new());
    let agents = Arc::new(RwLock::new(Vec::new()));
    let state = AppState::new(
        Arc::new(config),
        None,
        Arc::new(ServiceState::default()),
        store.clone(),
        Arc::new(MockJobEngine::new()),
        None,
        agents.clone(),
    );

    TestStateHandles {
        state,
        store,
        agents,
    }
}
