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

#[cfg(feature = "bundled-frontend")]
mod endpoint {
    use crate::{
        app::{AppState, ServiceState},
        config::AuthConfig,
        routes::local_auth::LocalAuthResponse,
        setup_local_auth,
        store::MemoryStore,
        test_utils::{
            MockJobEngine, spawn_test_server_with_state, test_app_config, test_client_without_auth,
            test_secret_manager,
        },
    };
    use reqwest::StatusCode;
    use std::sync::Arc;

    fn local_auth_state(
        config: crate::config::AppConfig,
    ) -> (AppState, Arc<dyn crate::store::Store>) {
        let store: Arc<dyn crate::store::Store> = Arc::new(MemoryStore::new());
        let state = AppState::new(
            Arc::new(config),
            None,
            Arc::new(ServiceState::default()),
            store.clone(),
            Arc::new(MockJobEngine::new()),
            test_secret_manager(),
        );
        (state, store)
    }

    #[tokio::test]
    async fn local_auth_returns_token() -> anyhow::Result<()> {
        let tmp_dir = tempfile::TempDir::new()?;
        let token_path = tmp_dir.path().join("auth-token");

        let mut config = test_app_config();
        config.auth = AuthConfig::Local {
            github_token: "ghp_test_token".to_string(),
            username: None,
            auth_token_file: Some(token_path.clone()),
        };

        let (state, store) = local_auth_state(config.clone());
        setup_local_auth(&config, store.as_ref()).await?;

        let server = spawn_test_server_with_state(state, store).await?;
        let client = test_client_without_auth();

        let response = client
            .get(format!("{}/v1/local-auth", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), StatusCode::OK);

        let body: LocalAuthResponse = response.json().await?;
        assert!(!body.token.is_empty());

        // Token should match the file contents (trimmed).
        let file_token = std::fs::read_to_string(&token_path)?.trim().to_string();
        assert_eq!(body.token, file_token);

        Ok(())
    }

    #[tokio::test]
    async fn local_auth_returns_400_when_not_configured() -> anyhow::Result<()> {
        // Use default config which has Github auth (no auth_token_file).
        let config = test_app_config();
        let (state, store) = local_auth_state(config);

        let server = spawn_test_server_with_state(state, store).await?;
        let client = test_client_without_auth();

        let response = client
            .get(format!("{}/v1/local-auth", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        Ok(())
    }

    #[tokio::test]
    async fn local_auth_returns_400_when_file_missing() -> anyhow::Result<()> {
        let mut config = test_app_config();
        config.auth = AuthConfig::Local {
            github_token: "ghp_test_token".to_string(),
            username: None,
            auth_token_file: Some("/tmp/nonexistent-auth-token-file".into()),
        };

        let (state, store) = local_auth_state(config);

        let server = spawn_test_server_with_state(state, store).await?;
        let client = test_client_without_auth();

        let response = client
            .get(format!("{}/v1/local-auth", server.base_url()))
            .send()
            .await?;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        Ok(())
    }
}
