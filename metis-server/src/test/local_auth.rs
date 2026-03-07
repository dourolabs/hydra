use crate::{
    config::AuthMode,
    domain::{actors::AuthToken, users::Username},
    setup_local_auth,
    store::{MemoryStore, ReadOnlyStore},
    test_utils::test_app_config,
};
use std::sync::Arc;

#[tokio::test]
async fn setup_local_auth_creates_actor_and_writes_token() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let token_path = temp_dir.path().join("auth-token");

    let mut config = test_app_config();
    config.auth_mode = AuthMode::Local;

    let store = Arc::new(MemoryStore::new());
    setup_local_auth(&config, store.as_ref(), &token_path).await?;

    // Token file should exist and be non-empty.
    let token_contents = std::fs::read_to_string(&token_path)?;
    assert!(!token_contents.trim().is_empty());

    // Token should parse as a valid auth token with actor name "u-local".
    let auth_token =
        AuthToken::parse(token_contents.trim()).expect("token should parse as valid auth token");
    assert_eq!(auth_token.actor_name(), "u-local");

    // Actor should exist in the store.
    let actor = store.as_ref().get_actor("u-local").await?;
    assert!(actor.item.verify_auth_token(&auth_token));

    Ok(())
}

#[tokio::test]
async fn setup_local_auth_is_idempotent() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let token_path = temp_dir.path().join("auth-token");

    let mut config = test_app_config();
    config.auth_mode = AuthMode::Local;

    let store = Arc::new(MemoryStore::new());

    // Run twice — second call should not fail.
    setup_local_auth(&config, store.as_ref(), &token_path).await?;
    let first_token = std::fs::read_to_string(&token_path)?;

    setup_local_auth(&config, store.as_ref(), &token_path).await?;
    let second_token = std::fs::read_to_string(&token_path)?;

    // Both should be valid tokens (they may differ since new tokens are generated).
    let parsed = AuthToken::parse(second_token.trim()).expect("second token should parse");
    assert_eq!(parsed.actor_name(), "u-local");

    // The actor in the store should match the latest token.
    let actor = store.as_ref().get_actor("u-local").await?;
    assert!(actor.item.verify_auth_token(&parsed));

    // First token should no longer work (actor was updated).
    if first_token != second_token {
        let old_parsed = AuthToken::parse(first_token.trim()).unwrap();
        assert!(!actor.item.verify_auth_token(&old_parsed));
    }

    Ok(())
}

#[tokio::test]
async fn setup_local_auth_stores_github_pat() -> anyhow::Result<()> {
    let temp_dir = tempfile::tempdir()?;
    let token_path = temp_dir.path().join("auth-token");

    let mut config = test_app_config();
    config.auth_mode = AuthMode::Local;
    config.github_token = Some("ghp_test_pat_token_123".to_string());

    let store = Arc::new(MemoryStore::new());
    setup_local_auth(&config, store.as_ref(), &token_path).await?;

    // User should exist with the GitHub token.
    let username = Username::from("local");
    let user = store.as_ref().get_user(&username, false).await?;
    assert_eq!(user.item.github_token, "ghp_test_pat_token_123");
    assert!(user.item.github_refresh_token.is_empty());

    Ok(())
}
