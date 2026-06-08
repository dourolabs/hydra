//! Regression test for [[i-lnmjbjxk]] — multi-user patch-author attribution.
//!
//! Before the creator denormalization, every `agents/<name>` session shared
//! a single `actors_v2` row whose `creator` column was pinned to the first
//! user who ever spawned the agent. Two sessions for two different users
//! both attributed patch creation back to that pinned creator (typically
//! `jayantk`). RCA: [[d-miqcehb]].
//!
//! After the fix, `auth_tokens.creator` is the per-token denormalization
//! of the originating session's creator — the auth middleware builds the
//! runtime `Actor` directly off the token row, so two distinct session
//! tokens for the same `ActorId::Agent` resolve to distinct creators.
//!
//! Test shape, per the issue's "Regression test (must include)" section:
//!
//! - Real `SqliteStore`, not `MemoryStore` (the latter mirrors the struct
//!   as-is and would trivially pass even with the bug in place).
//! - Two distinct users (`alice`, `bob`).
//! - Spawn one `agents/swe` session under each, mint each session's
//!   auth token via `create_actor_for_job`, then drive a patch upsert
//!   request as that session and assert `patch.creator == <expected>`.
//! - A live `gh pr view` check is out of scope. The github_pr_sync
//!   integration calls Octocrab's `personal_token` flow with the
//!   patch creator's GitHub PAT — the upstream attribution follows
//!   from `patch.creator` directly, which is what we assert here.
//!
//! Lives in `tests/` (rather than `src/test/*`) so it picks up the real
//! `SqliteStore` migration plan (including the new
//! `20260609000000_add_creator_to_auth_tokens` migration) rather than
//! the in-process `MemoryStore` happy path.

use anyhow::{Context, Result};
use hydra_common::RepoName;
use hydra_common::api::v1::agents::AgentName;
use hydra_common::api::v1::patches::UpsertPatchRequest;
use hydra_common::api::v1::sessions::Bundle;
use hydra_common::repositories::Repository;
use hydra_server::app::AppState;
use hydra_server::domain::actors::{Actor, ActorRef, AuthToken};
use hydra_server::domain::patches::{Patch, PatchStatus};
use hydra_server::domain::sessions::{AgentConfig, Session, SessionMode};
use hydra_server::domain::task_status::Status;
use hydra_server::domain::users::{User, Username};
use hydra_server::routes::sessions::mount_spec_from_create_request;
use hydra_server::store::Store;
use hydra_server::store::sqlite_store::{self, SqliteStore};
use hydra_server::test_utils::test_state_with_store;
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::test]
async fn distinct_session_tokens_resolve_to_distinct_creators_via_auth_middleware() -> Result<()> {
    let pool = SqliteStore::init_pool("sqlite::memory:")
        .await
        .context("init in-memory sqlite pool")?;
    sqlite_store::run_migrations(&pool, None)
        .await
        .context("apply sqlite migrations to HEAD")?;
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool));
    let handles = test_state_with_store(store.clone());

    let alice = Username::from("alice");
    let bob = Username::from("bob");

    store
        .add_user(User::new(alice.clone(), None, false), &ActorRef::test())
        .await?;
    store
        .add_user(User::new(bob.clone(), None, false), &ActorRef::test())
        .await?;

    let agent_name = AgentName::try_new("swe").unwrap();

    // Distinct sessions for the same shared `agents/swe` actor identity,
    // each created by a different user. Pre-fix, both would attribute
    // any patch they create to whichever user instantiated the
    // `agents/swe` actor row first.
    let alice_token =
        spawn_agent_session_and_mint_token(&handles.state, store.as_ref(), &agent_name, &alice)
            .await?;
    let bob_token =
        spawn_agent_session_and_mint_token(&handles.state, store.as_ref(), &agent_name, &bob)
            .await?;

    // The two tokens carry the same actor name (`agents/swe`) but
    // different originating creators — this is the exact shape that
    // pre-fix collapsed into a single creator at the actor table.
    let alice_parsed = AuthToken::parse(&alice_token)?;
    let bob_parsed = AuthToken::parse(&bob_token)?;
    assert_eq!(alice_parsed.actor_name(), "agents/swe");
    assert_eq!(bob_parsed.actor_name(), "agents/swe");

    let alice_row = handles
        .state
        .store()
        .get_auth_token_by_hash(&Actor::hash_auth_token(alice_parsed.raw_token()))
        .await?
        .context("alice token must be retrievable by hash")?;
    let bob_row = handles
        .state
        .store()
        .get_auth_token_by_hash(&Actor::hash_auth_token(bob_parsed.raw_token()))
        .await?
        .context("bob token must be retrievable by hash")?;

    // Domain-level assertion: the auth-middleware-equivalent lookup
    // returns the *per-token* creator, not the agent row's creator.
    // This is the load-bearing variable the RCA identified: pre-fix,
    // both rows would carry the same creator.
    assert_eq!(
        alice_row.creator, alice,
        "alice's token must resolve to alice"
    );
    assert_eq!(bob_row.creator, bob, "bob's token must resolve to bob");
    assert_ne!(
        alice_row.creator, bob_row.creator,
        "two sessions for the same agent identity must NOT share a creator (this is the bug)",
    );

    // End-to-end through the patch upsert path: build the same `Actor`
    // shape the auth middleware would have stamped onto the request,
    // then drive an upsert through `AppState::upsert_patch_from_request`.
    // The persisted `patch.creator` must come back as the session's
    // originating user, not the other one.
    let repo_name = seed_repository(store.as_ref()).await?;

    let alice_patch_id = upsert_patch_as(
        &handles.state,
        &alice_row,
        alice_parsed.actor_name(),
        &repo_name,
        "alice patch",
    )
    .await?;
    let bob_patch_id = upsert_patch_as(
        &handles.state,
        &bob_row,
        bob_parsed.actor_name(),
        &repo_name,
        "bob patch",
    )
    .await?;

    let alice_patch = store.get_patch(&alice_patch_id, false).await?;
    let bob_patch = store.get_patch(&bob_patch_id, false).await?;

    assert_eq!(
        alice_patch.item.creator, alice,
        "alice's patch must attribute to alice"
    );
    assert_eq!(
        bob_patch.item.creator, bob,
        "bob's patch must attribute to bob"
    );

    Ok(())
}

/// Persist a session whose `creator = creator` and whose agent is the
/// shared `agents/<name>` actor, then mint the session's auth token via
/// the same `create_actor_for_job` path the live `sessions` route uses.
async fn spawn_agent_session_and_mint_token(
    state: &AppState,
    store: &dyn Store,
    agent_name: &AgentName,
    creator: &Username,
) -> Result<String> {
    let session = Session::new(
        creator.clone(),
        None,
        None,
        AgentConfig::new(Some(agent_name.clone()), None, None, None),
        mount_spec_from_create_request(Bundle::None, None),
        None,
        HashMap::new(),
        None,
        None,
        None,
        SessionMode::Headless,
        Status::Created,
        None,
        None,
    );
    let (session_id, _) = store
        .add_session(session, chrono::Utc::now(), &ActorRef::test())
        .await?;
    let (_actor, auth_token) = state.create_actor_for_job(session_id).await?;
    Ok(auth_token)
}

async fn seed_repository(store: &dyn Store) -> Result<RepoName> {
    let name = RepoName::new("dourolabs", "hydra").unwrap();
    let repo = Repository::new(
        "https://github.com/dourolabs/hydra".to_string(),
        Some("main".to_string()),
        None,
    );
    store
        .add_repository(name.clone(), repo, &ActorRef::test())
        .await?;
    Ok(name)
}

async fn upsert_patch_as(
    state: &AppState,
    auth_row: &hydra_server::store::AuthTokenRow,
    actor_name: &str,
    repo_name: &RepoName,
    title: &str,
) -> Result<hydra_common::PatchId> {
    // Mirrors `routes/auth.rs::require_auth`: build the runtime `Actor`
    // straight from the matched `auth_tokens` row. The auth middleware
    // is the load-bearing step — pre-fix, this would have produced an
    // `Actor` whose `creator` was the shared agent row's creator
    // regardless of which session's token came in.
    let actor_id =
        Actor::parse_name(actor_name).context("parse actor_name into ActorId for upsert")?;
    let actor = Actor {
        actor_id,
        creator: auth_row.creator.clone(),
        session_id: auth_row.session_id.clone(),
    };

    // `routes/patches::create_patch` requires the request's `creator`
    // to equal `actor.creator` — see hydra-server/src/routes/patches.rs.
    // Stamp the request to mirror what a real CLI / web client would
    // send for the authenticated user; the assertion above on
    // `auth_row.creator` is what proves the per-token denormalization
    // is correct, and the patch round-trip here proves the same value
    // survives all the way to `patches.creator` in the store.
    let patch = Patch::new(
        title.to_string(),
        "regression test".to_string(),
        "diff".to_string(),
        PatchStatus::Open,
        false,
        actor.creator.clone(),
        Vec::new(),
        repo_name.clone(),
        None,
        None,
        None,
        None,
    );
    let request = UpsertPatchRequest::new(patch.into());
    let (patch_id, _) = state
        .upsert_patch_from_request(ActorRef::from(&actor), None, request)
        .await?;
    Ok(patch_id)
}
