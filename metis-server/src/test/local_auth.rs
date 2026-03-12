use crate::{
    config::AuthConfig,
    domain::{actors::AuthToken, users::Username},
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
        auth_token_file: None,
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
        auth_token_file: None,
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
        auth_token_file: None,
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
        auth_token_file: None,
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

#[tokio::test]
async fn setup_local_auth_writes_token_file() -> anyhow::Result<()> {
    let tmp_dir = tempfile::TempDir::new()?;
    let token_path = tmp_dir.path().join("auth-token");

    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_test_token".to_string(),
        username: None,
        auth_token_file: Some(token_path.clone()),
    };

    let store = Arc::new(MemoryStore::new());
    setup_local_auth(&config, store.as_ref()).await?;

    // Token file should exist and be non-empty.
    let token_contents = std::fs::read_to_string(&token_path)?;
    assert!(!token_contents.is_empty());

    // The file contents should be a valid auth token that matches the stored actor.
    let actor = store.as_ref().get_actor("u-local").await?;
    let parsed_token = AuthToken::parse(&token_contents)?;
    assert!(actor.item.verify_auth_token(&parsed_token));

    // On Unix, verify the file permissions are 0o600.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = std::fs::metadata(&token_path)?;
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
    }

    Ok(())
}

/// Calling setup_local_auth twice with the same persistent store should reuse
/// the existing actor and leave the token file unchanged.
#[tokio::test]
async fn setup_local_auth_preserves_token_across_restart() -> anyhow::Result<()> {
    let tmp_dir = tempfile::TempDir::new()?;
    let token_path = tmp_dir.path().join("auth-token");

    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_test_token".to_string(),
        username: None,
        auth_token_file: Some(token_path.clone()),
    };

    // First call: creates actor and writes token file.
    let store = Arc::new(MemoryStore::new());
    setup_local_auth(&config, store.as_ref()).await?;

    let token_after_first = std::fs::read_to_string(&token_path)?;
    assert!(!token_after_first.is_empty());

    // Second call with the SAME store: actor exists, so setup is skipped
    // and the token file remains unchanged.
    setup_local_auth(&config, store.as_ref()).await?;

    let token_after_second = std::fs::read_to_string(&token_path)?;
    assert_eq!(
        token_after_first, token_after_second,
        "auth token should be stable across server restarts"
    );

    // The actor in the store should still verify the original token.
    let actor = store.as_ref().get_actor("u-local").await?;
    let parsed = AuthToken::parse(&token_after_first)?;
    assert!(
        actor.item.verify_auth_token(&parsed),
        "actor should verify the persisted token after restart"
    );

    Ok(())
}
