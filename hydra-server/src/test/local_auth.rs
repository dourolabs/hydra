use crate::{
    config::AuthConfig,
    domain::{
        actors::{Actor, AuthToken},
        secrets::{SECRET_CLAUDE_CODE_OAUTH_TOKEN, SECRET_GITHUB_TOKEN, SecretManager},
        users::Username,
    },
    setup_local_auth,
    store::{MemoryStore, ReadOnlyStore, Store},
    test_utils::test_app_config,
};
use std::sync::Arc;

/// Resolve `auth_token` against the post-Phase-3b auth path: look up the
/// `auth_tokens` row by hash and confirm it points at the expected actor.
async fn token_is_registered_for(
    store: &dyn ReadOnlyStore,
    actor_name: &str,
    auth_token: &str,
) -> bool {
    let parsed = match AuthToken::parse(auth_token) {
        Ok(t) => t,
        Err(_) => return false,
    };
    if parsed.actor_name() != actor_name {
        return false;
    }
    let hash = Actor::hash_auth_token(parsed.raw_token());
    match store.get_auth_token_by_hash(&hash).await {
        Ok(Some(row)) => row.actor_name == actor_name && !row.is_revoked,
        _ => false,
    }
}

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

    // Local auth seeds a `users/<name>` row plus an `auth_tokens` row
    // pointing at that actor name (the actors table is gone — the auth
    // middleware reconstructs the runtime `Actor` from the token row).
    let hashes = store.get_auth_token_hashes("users/local").await?;
    assert_eq!(hashes.len(), 1, "expected exactly one minted token");

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

    // Token rotates on each call — after the second run there is exactly
    // one token registered for the local user.
    let hashes = store.get_auth_token_hashes("users/local").await?;
    assert_eq!(hashes.len(), 1, "expected exactly one minted token");

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

    // A token should be registered under the custom actor name.
    let hashes = store.get_auth_token_hashes("users/alice").await?;
    assert_eq!(hashes.len(), 1, "expected exactly one minted token");

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

    // The file contents should be a valid auth token registered in the store.
    assert!(token_is_registered_for(store.as_ref(), "users/local", &token_contents).await);

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

    // The new token should be registered for the actor in the auth_tokens table.
    assert!(
        token_is_registered_for(store.as_ref(), "users/local", &token_after_second).await,
        "actor should verify the newly generated token"
    );

    // The first token must no longer authenticate — `setup_local_auth`
    // rotates by deleting the actor's previous `auth_tokens` rows on
    // re-init.
    assert!(
        !token_is_registered_for(store.as_ref(), "users/local", &token_after_first).await,
        "previous token should be invalidated after re-init"
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
    std::fs::write(&token_path, "users/local:stale-token-value")?;

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
        token, "users/local:stale-token-value",
        "stale token should be replaced"
    );

    assert!(
        token_is_registered_for(store.as_ref(), "users/local", &token).await,
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

    assert!(
        token_is_registered_for(store.as_ref(), "users/local", &token).await,
        "actor should verify the recreated token"
    );

    Ok(())
}

#[tokio::test]
async fn setup_local_auth_rejects_empty_claude_oauth_token() {
    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_test_token".to_string(),
        username: None,
        auth_token_file: None,
    };
    config.hydra.claude_code_oauth_token = Some(String::new());

    let store = Arc::new(MemoryStore::new());
    let sm = test_secret_manager();
    let err = setup_local_auth(&config, store.as_ref(), &sm)
        .await
        .expect_err("empty CLAUDE_CODE_OAUTH_TOKEN should be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("claude_code_oauth_token"),
        "error message should reference the offending config key, got: {msg}"
    );
}

#[tokio::test]
async fn setup_local_auth_rejects_whitespace_claude_oauth_token() {
    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_test_token".to_string(),
        username: None,
        auth_token_file: None,
    };
    config.hydra.claude_code_oauth_token = Some("   \t\n".to_string());

    let store = Arc::new(MemoryStore::new());
    let sm = test_secret_manager();
    let err = setup_local_auth(&config, store.as_ref(), &sm)
        .await
        .expect_err("whitespace-only CLAUDE_CODE_OAUTH_TOKEN should be rejected");
    let msg = format!("{err}");
    assert!(
        msg.contains("claude_code_oauth_token"),
        "error message should reference the offending config key, got: {msg}"
    );
}

#[tokio::test]
async fn setup_local_auth_accepts_missing_claude_oauth_token() -> anyhow::Result<()> {
    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_test_token".to_string(),
        username: None,
        auth_token_file: None,
    };
    config.hydra.claude_code_oauth_token = None;

    let store = Arc::new(MemoryStore::new());
    let sm = test_secret_manager();
    setup_local_auth(&config, store.as_ref(), &sm).await?;

    let username = Username::from("local");
    let stored = store
        .get_user_secret(&username, SECRET_CLAUDE_CODE_OAUTH_TOKEN)
        .await?;
    assert!(
        stored.is_none(),
        "no CLAUDE_CODE_OAUTH_TOKEN should be stored when config field is absent"
    );

    Ok(())
}

#[tokio::test]
async fn setup_local_auth_stores_valid_claude_oauth_token() -> anyhow::Result<()> {
    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_test_token".to_string(),
        username: None,
        auth_token_file: None,
    };
    config.hydra.claude_code_oauth_token = Some("oauth-real-token".to_string());

    let store = Arc::new(MemoryStore::new());
    let sm = test_secret_manager();
    setup_local_auth(&config, store.as_ref(), &sm).await?;

    let username = Username::from("local");
    let encrypted = store
        .get_user_secret(&username, SECRET_CLAUDE_CODE_OAUTH_TOKEN)
        .await?
        .expect("CLAUDE_CODE_OAUTH_TOKEN should be stored");
    assert_eq!(sm.decrypt(&encrypted)?, "oauth-real-token");

    Ok(())
}

/// A previously-stored CLAUDE_CODE_OAUTH_TOKEN (e.g. from a misconfigured prior
/// run that wrote an empty value before validation existed) is overwritten by
/// the validated config value on the next setup_local_auth call.
#[tokio::test]
async fn setup_local_auth_overwrites_stale_empty_claude_oauth_token() -> anyhow::Result<()> {
    let store = Arc::new(MemoryStore::new());
    let sm = test_secret_manager();
    let username = Username::from("local");

    // Seed the store with a stale empty value, as p-rersye's downstream-skip
    // workaround had to compensate for.
    let stale = sm.encrypt("")?;
    store
        .set_user_secret(&username, SECRET_CLAUDE_CODE_OAUTH_TOKEN, &stale, true)
        .await?;

    let mut config = test_app_config();
    config.auth = AuthConfig::Local {
        github_token: "ghp_test_token".to_string(),
        username: None,
        auth_token_file: None,
    };
    config.hydra.claude_code_oauth_token = Some("oauth-real-token".to_string());

    setup_local_auth(&config, store.as_ref(), &sm).await?;

    let encrypted = store
        .get_user_secret(&username, SECRET_CLAUDE_CODE_OAUTH_TOKEN)
        .await?
        .expect("CLAUDE_CODE_OAUTH_TOKEN should be stored");
    assert_eq!(
        sm.decrypt(&encrypted)?,
        "oauth-real-token",
        "stale empty row should have been overwritten by the validated config value"
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
