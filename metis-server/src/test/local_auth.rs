use crate::{
    config::AuthConfig,
    domain::users::Username,
    setup_local_auth,
    store::{MemoryStore, ReadOnlyStore},
    test_utils::test_app_config,
};
use std::sync::Arc;

#[tokio::test]
async fn setup_local_auth_creates_actor() -> anyhow::Result<()> {
    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_test_token".to_string(),
        username: None,
    };

    let store = Arc::new(MemoryStore::new());
    setup_local_auth(&config, store.as_ref()).await?;

    // Actor should exist in the store.
    let actor = store.as_ref().get_actor("u-local").await?;
    assert_eq!(actor.item.name(), "u-local");

    Ok(())
}

#[tokio::test]
async fn setup_local_auth_is_idempotent() -> anyhow::Result<()> {
    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_test_token".to_string(),
        username: None,
    };

    let store = Arc::new(MemoryStore::new());

    // Run twice — second call should not fail.
    setup_local_auth(&config, store.as_ref()).await?;
    setup_local_auth(&config, store.as_ref()).await?;

    // The actor in the store should still exist.
    let actor = store.as_ref().get_actor("u-local").await?;
    assert_eq!(actor.item.name(), "u-local");

    Ok(())
}

#[tokio::test]
async fn setup_local_auth_stores_github_pat() -> anyhow::Result<()> {
    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_test_pat_token_123".to_string(),
        username: None,
    };

    let store = Arc::new(MemoryStore::new());
    setup_local_auth(&config, store.as_ref()).await?;

    // User should exist.
    let username = Username::from("local");
    let user = store.as_ref().get_user(&username, false).await?;
    assert_eq!(user.item.username, username);

    Ok(())
}

#[tokio::test]
async fn setup_local_auth_uses_custom_username() -> anyhow::Result<()> {
    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_test_token".to_string(),
        username: Some("alice".to_string()),
    };

    let store = Arc::new(MemoryStore::new());
    setup_local_auth(&config, store.as_ref()).await?;

    // Actor should exist under the custom username.
    let actor = store.as_ref().get_actor("u-alice").await?;
    assert_eq!(actor.item.name(), "u-alice");

    // User should also be stored under the custom username.
    let username = Username::from("alice");
    let user = store.as_ref().get_user(&username, false).await?;
    assert_eq!(user.item.username, username);

    Ok(())
}
