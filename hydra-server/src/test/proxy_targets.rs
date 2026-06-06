//! Integration tests for `/v1/sessions/:session_id/proxy-targets`.
//!
//! These cover:
//! - Round-trip: a worker-scoped POST followed by GET returns the same
//!   `ProxyTarget`. The same store impl powers both the sqlite and
//!   Postgres v2 backends (the postgres test variant ignored by default
//!   lives next to the sqlite store; this test pins the route-level
//!   behaviour on the default-backed test harness).
//! - Worker-auth gating: a worker authenticated for session `A` may not
//!   POST against session `B`'s `/proxy-targets` — the server returns
//!   `403`.

use crate::domain::actors::Actor;
use crate::domain::sessions::SessionMode;
use crate::domain::users::Username;
use crate::store::Session;
use crate::test_utils::{
    register_actor_and_token, spawn_test_server_with_state, test_client, test_state_handles,
};
use hydra_common::api::v1::sessions::{ListProxyTargetsResponse, UpsertProxyTargetRequest};
use hydra_common::{ActorId, ActorRef};
use reqwest::{StatusCode, header};
use std::collections::HashMap;

fn make_headless_session(creator: &str) -> Session {
    let mount_spec = crate::routes::sessions::mount_spec_from_create_request(
        hydra_common::api::v1::sessions::Bundle::None,
        None,
    );
    Session::new(
        Username::from(creator),
        None,
        None,
        Default::default(),
        mount_spec,
        None,
        HashMap::new(),
        None,
        None,
        None,
        SessionMode::Headless,
        crate::store::Status::Created,
        None,
        None,
    )
}

#[tokio::test]
async fn proxy_targets_round_trip_via_routes() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let store = handles.store.clone();
    let server = spawn_test_server_with_state(handles.state, store.clone()).await?;

    let (sid, _) = store
        .add_session(
            make_headless_session("rt-creator"),
            chrono::Utc::now(),
            &ActorRef::test(),
        )
        .await?;

    // Worker auth: actor bound to this session's id.
    let (actor, auth_token) = Actor::new_from_actor_id(
        ActorId::Adhoc(sid.clone()),
        Username::from("rt-worker"),
        None,
    );
    register_actor_and_token(store.as_ref(), &actor, &auth_token, Some(&sid)).await?;

    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_str(&format!("Bearer {auth_token}"))?,
    );
    let worker_client = reqwest::Client::builder()
        .default_headers(headers)
        .build()?;

    // POST records a target.
    let post = worker_client
        .post(format!(
            "{}/v1/sessions/{sid}/proxy-targets",
            server.base_url()
        ))
        .json(&UpsertProxyTargetRequest {
            port: 3000,
            ready_path: Some("/ready".to_string()),
        })
        .send()
        .await?;
    assert_eq!(post.status(), StatusCode::NO_CONTENT);

    // GET returns the same target. Use the user-auth `test_client` so the
    // read path doesn't accidentally require worker auth.
    let read_client = test_client();
    let list: ListProxyTargetsResponse = read_client
        .get(format!(
            "{}/v1/sessions/{sid}/proxy-targets",
            server.base_url()
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(list.targets.len(), 1);
    assert_eq!(list.targets[0].port, 3000);
    assert_eq!(list.targets[0].ready_path.as_deref(), Some("/ready"));

    // POST again replaces `ready_path` (idempotent on `port`).
    let post2 = worker_client
        .post(format!(
            "{}/v1/sessions/{sid}/proxy-targets",
            server.base_url()
        ))
        .json(&UpsertProxyTargetRequest {
            port: 3000,
            ready_path: None,
        })
        .send()
        .await?;
    assert_eq!(post2.status(), StatusCode::NO_CONTENT);
    let list2: ListProxyTargetsResponse = read_client
        .get(format!(
            "{}/v1/sessions/{sid}/proxy-targets",
            server.base_url()
        ))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(list2.targets.len(), 1);
    assert_eq!(list2.targets[0].ready_path, None);

    // DELETE removes the target; calling DELETE again is a no-op (idempotent).
    let del = worker_client
        .delete(format!(
            "{}/v1/sessions/{sid}/proxy-targets/3000",
            server.base_url()
        ))
        .send()
        .await?;
    assert_eq!(del.status(), StatusCode::NO_CONTENT);
    let del2 = worker_client
        .delete(format!(
            "{}/v1/sessions/{sid}/proxy-targets/3000",
            server.base_url()
        ))
        .send()
        .await?;
    assert_eq!(del2.status(), StatusCode::NO_CONTENT);

    let list3: ListProxyTargetsResponse = read_client
        .get(format!(
            "{}/v1/sessions/{sid}/proxy-targets",
            server.base_url()
        ))
        .send()
        .await?
        .json()
        .await?;
    assert!(list3.targets.is_empty());

    Ok(())
}

#[tokio::test]
async fn worker_for_session_a_cannot_post_against_session_b() -> anyhow::Result<()> {
    let handles = test_state_handles();
    let store = handles.store.clone();
    let server = spawn_test_server_with_state(handles.state, store.clone()).await?;

    let (sid_a, _) = store
        .add_session(
            make_headless_session("creator-a"),
            chrono::Utc::now(),
            &ActorRef::test(),
        )
        .await?;
    let (sid_b, _) = store
        .add_session(
            make_headless_session("creator-b"),
            chrono::Utc::now(),
            &ActorRef::test(),
        )
        .await?;

    // Worker auth bound to session A.
    let (actor_a, token_a) = Actor::new_from_actor_id(
        ActorId::Adhoc(sid_a.clone()),
        Username::from("worker-a"),
        None,
    );
    register_actor_and_token(store.as_ref(), &actor_a, &token_a, Some(&sid_a)).await?;

    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::AUTHORIZATION,
        header::HeaderValue::from_str(&format!("Bearer {token_a}"))?,
    );
    let worker_a_client = reqwest::Client::builder()
        .default_headers(headers)
        .build()?;

    // Worker A trying to POST against session B must be refused.
    let cross = worker_a_client
        .post(format!(
            "{}/v1/sessions/{sid_b}/proxy-targets",
            server.base_url()
        ))
        .json(&UpsertProxyTargetRequest {
            port: 4000,
            ready_path: None,
        })
        .send()
        .await?;
    assert!(
        cross.status() == StatusCode::FORBIDDEN || cross.status() == StatusCode::UNAUTHORIZED,
        "expected 403/401, got {}",
        cross.status()
    );

    // Worker A targeting its own session must succeed.
    let own = worker_a_client
        .post(format!(
            "{}/v1/sessions/{sid_a}/proxy-targets",
            server.base_url()
        ))
        .json(&UpsertProxyTargetRequest {
            port: 4000,
            ready_path: None,
        })
        .send()
        .await?;
    assert_eq!(own.status(), StatusCode::NO_CONTENT);

    Ok(())
}
