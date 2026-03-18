use crate::{
    config::AuthConfig,
    domain::{
        actors::AuthToken,
        secrets::{SECRET_GITHUB_TOKEN, SecretManager},
        users::Username,
    },
    setup_local_auth,
    store::{MemoryStore, ReadOnlyStore},
    test_utils::test_app_config,
};
use std::sync::Arc;

fn test_secret_manager() -> SecretManager {
    SecretManager::new([42u8; 32])
}

#[tokio::test]
async fn setup_local_auth_creates_actor() -> anyhow::Result<()> {
    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_test_token".to_string(),
        username: None,
        auth_token_file: None,
    };

    let store = Arc::new(MemoryStore::new());
    let sm = test_secret_manager();
    setup_local_auth(&config, store.as_ref(), &sm).await?;

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
    let sm = test_secret_manager();

    // Run twice — second call should not fail.
    setup_local_auth(&config, store.as_ref(), &sm).await?;
    setup_local_auth(&config, store.as_ref(), &sm).await?;

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
    let sm = test_secret_manager();
    setup_local_auth(&config, store.as_ref(), &sm).await?;

    // User should exist.
    let username = Username::from("local");
    let user = store.as_ref().get_user(&username, false).await?;
    assert_eq!(user.item.username, username);

    // The GitHub token should be stored encrypted and decryptable.
    let encrypted = store
        .get_user_secret(&username, SECRET_GITHUB_TOKEN)
        .await?
        .expect("GITHUB_TOKEN secret should exist");
    let decrypted = sm.decrypt(&encrypted)?;
    assert_eq!(decrypted, "ghp_test_pat_token_123");

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
    let sm = test_secret_manager();
    setup_local_auth(&config, store.as_ref(), &sm).await?;

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
    let sm = test_secret_manager();
    setup_local_auth(&config, store.as_ref(), &sm).await?;

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

/// Calling setup_local_auth twice with the same persistent store regenerates
/// the token and updates the actor hash so the new token file is always valid.
#[tokio::test]
async fn setup_local_auth_regenerates_token_on_reinit() -> anyhow::Result<()> {
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
    let sm = test_secret_manager();
    setup_local_auth(&config, store.as_ref(), &sm).await?;

    let token_after_first = std::fs::read_to_string(&token_path)?;
    assert!(!token_after_first.is_empty());

    // Second call with the SAME store: actor exists, token is regenerated.
    setup_local_auth(&config, store.as_ref(), &sm).await?;

    let token_after_second = std::fs::read_to_string(&token_path)?;
    assert!(
        !token_after_second.is_empty(),
        "token file should be non-empty after re-init"
    );

    // The new token should verify against the updated actor hash.
    let actor = store.as_ref().get_actor("u-local").await?;
    let parsed_new = AuthToken::parse(&token_after_second)?;
    assert!(
        actor.item.verify_auth_token(&parsed_new),
        "actor should verify the newly generated token"
    );

    Ok(())
}

/// When a stale token file exists but the actor is gone (e.g., DB was deleted),
/// setup_local_auth creates a fresh actor and overwrites the token file.
#[tokio::test]
async fn setup_local_auth_overwrites_stale_token_file() -> anyhow::Result<()> {
    let tmp_dir = tempfile::TempDir::new()?;
    let token_path = tmp_dir.path().join("auth-token");

    // Simulate a stale token file from a previous run.
    std::fs::write(&token_path, "u-local:stale-token-value")?;

    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_test_token".to_string(),
        username: None,
        auth_token_file: Some(token_path.clone()),
    };

    let store = Arc::new(MemoryStore::new());
    let sm = test_secret_manager();
    setup_local_auth(&config, store.as_ref(), &sm).await?;

    // The token file should have been overwritten with a valid token.
    let token = std::fs::read_to_string(&token_path)?;
    assert_ne!(
        token, "u-local:stale-token-value",
        "stale token should be replaced"
    );

    let actor = store.as_ref().get_actor("u-local").await?;
    let parsed = AuthToken::parse(&token)?;
    assert!(
        actor.item.verify_auth_token(&parsed),
        "actor should verify the fresh token that replaced the stale one"
    );

    Ok(())
}

/// When the actor exists in the DB but the token file was deleted,
/// setup_local_auth regenerates the token and writes a new file.
#[tokio::test]
async fn setup_local_auth_recreates_deleted_token_file() -> anyhow::Result<()> {
    let tmp_dir = tempfile::TempDir::new()?;
    let token_path = tmp_dir.path().join("auth-token");

    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_test_token".to_string(),
        username: None,
        auth_token_file: Some(token_path.clone()),
    };

    let store = Arc::new(MemoryStore::new());
    let sm = test_secret_manager();

    // First call: creates actor and writes token file.
    setup_local_auth(&config, store.as_ref(), &sm).await?;
    assert!(token_path.exists());

    // Simulate token file deletion (e.g., user deleted config dir but DB persists).
    std::fs::remove_file(&token_path)?;
    assert!(!token_path.exists());

    // Second call: actor exists, token file is recreated.
    setup_local_auth(&config, store.as_ref(), &sm).await?;

    assert!(token_path.exists(), "token file should be recreated");
    let token = std::fs::read_to_string(&token_path)?;
    assert!(!token.is_empty());

    let actor = store.as_ref().get_actor("u-local").await?;
    let parsed = AuthToken::parse(&token)?;
    assert!(
        actor.item.verify_auth_token(&parsed),
        "actor should verify the recreated token"
    );

    Ok(())
}

#[tokio::test]
async fn setup_local_auth_updates_github_pat_on_config_change() -> anyhow::Result<()> {
    let store = Arc::new(MemoryStore::new());
    let sm = test_secret_manager();
    let username = Username::from("local");

    // First call with original token.
    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_original_token".to_string(),
        username: None,
        auth_token_file: None,
    };
    setup_local_auth(&config, store.as_ref(), &sm).await?;

    let encrypted = store
        .get_user_secret(&username, SECRET_GITHUB_TOKEN)
        .await?
        .expect("GITHUB_TOKEN should exist");
    assert_eq!(sm.decrypt(&encrypted)?, "ghp_original_token");

    // Second call with updated token (simulates config change between restarts).
    config.auth = AuthConfig::Local {
        github_token: "ghp_updated_token".to_string(),
        username: None,
        auth_token_file: None,
    };
    setup_local_auth(&config, store.as_ref(), &sm).await?;

    let encrypted = store
        .get_user_secret(&username, SECRET_GITHUB_TOKEN)
        .await?
        .expect("GITHUB_TOKEN should exist after update");
    assert_eq!(sm.decrypt(&encrypted)?, "ghp_updated_token");

    Ok(())
}
