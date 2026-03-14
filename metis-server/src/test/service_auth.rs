use crate::{
    domain::actors::AuthToken,
    setup_service_auth,
    store::{MemoryStore, ReadOnlyStore},
};
use std::sync::Arc;

#[tokio::test]
async fn setup_service_auth_creates_actor() -> anyhow::Result<()> {
    let store = Arc::new(MemoryStore::new());
    setup_service_auth(store.as_ref(), "bff", "my-secret-token").await?;

    let actor = store.as_ref().get_actor("svc-bff").await?;
    assert_eq!(actor.item.name(), "svc-bff");

    // The actor should verify with the correct token.
    let auth_token = AuthToken::parse("svc-bff:my-secret-token")?;
    assert!(actor.item.verify_auth_token(&auth_token));

    Ok(())
}

#[tokio::test]
async fn setup_service_auth_is_idempotent() -> anyhow::Result<()> {
    let store = Arc::new(MemoryStore::new());

    // Run twice with the same token — should not fail.
    setup_service_auth(store.as_ref(), "bff", "my-secret-token").await?;
    setup_service_auth(store.as_ref(), "bff", "my-secret-token").await?;

    let actor = store.as_ref().get_actor("svc-bff").await?;
    assert_eq!(actor.item.name(), "svc-bff");

    let auth_token = AuthToken::parse("svc-bff:my-secret-token")?;
    assert!(actor.item.verify_auth_token(&auth_token));

    Ok(())
}

#[tokio::test]
async fn setup_service_auth_updates_token_on_change() -> anyhow::Result<()> {
    let store = Arc::new(MemoryStore::new());

    // Create with original token.
    setup_service_auth(store.as_ref(), "bff", "original-token").await?;

    let actor = store.as_ref().get_actor("svc-bff").await?;
    let auth_token = AuthToken::parse("svc-bff:original-token")?;
    assert!(actor.item.verify_auth_token(&auth_token));

    // Update with new token.
    setup_service_auth(store.as_ref(), "bff", "updated-token").await?;

    let actor = store.as_ref().get_actor("svc-bff").await?;
    let new_auth_token = AuthToken::parse("svc-bff:updated-token")?;
    assert!(actor.item.verify_auth_token(&new_auth_token));

    // Old token should no longer work.
    let old_auth_token = AuthToken::parse("svc-bff:original-token")?;
    assert!(!actor.item.verify_auth_token(&old_auth_token));

    Ok(())
}

#[tokio::test]
async fn setup_service_auth_no_op_when_token_none() -> anyhow::Result<()> {
    // When bff_auth_token is None, setup_service_auth is not called and no
    // svc-bff actor should exist. We verify the store has no such actor.
    let store = Arc::new(MemoryStore::new());

    let result = store.as_ref().get_actor("svc-bff").await;
    assert!(result.is_err(), "svc-bff should not exist when not configured");

    Ok(())
}
