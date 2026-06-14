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
//! - Mint one auth token under each user's session, then resolve each
//!   token through `SqliteStore::get_auth_token_by_hash` (the lookup the
//!   auth middleware performs) and assert the per-token creator is the
//!   originating user. Drive `add_patch` through the runtime `Actor`
//!   built off that lookup and assert `patch.creator` matches.
//! - A live `gh pr view` check is out of scope. The github_pr_sync
//!   integration calls Octocrab's `personal_token` flow with the
//!   patch creator's GitHub PAT — the upstream attribution follows
//!   from `patch.creator` directly, which is what we assert here.
//!
//! Lives in `tests/` (rather than `src/test/*`) so it picks up the real
//! `SqliteStore` migration plan (including the new
//! `20260609000000_add_creator_to_auth_tokens` migration) rather than
//! the in-process `MemoryStore` happy path.
//!
//! Self-contained: avoids `hydra_server::test_utils` (which is gated on
//! the `test-utils` feature and would not be visible to this integration
//! crate under the workflow's `--features enterprise` build) by going
//! straight to the `Store` trait and the auth-token round-trip the
//! middleware uses.

use anyhow::{Context, Result};
use hydra_common::RepoName;
use hydra_common::api::v1::agents::AgentName;
use hydra_common::api::v1::sessions::MountSpec;
use hydra_common::repositories::Repository;
use hydra_server::domain::actors::{Actor, ActorId, ActorRef};
use hydra_server::domain::patches::{Patch, PatchStatus};
use hydra_server::domain::sessions::{AgentConfig, Session, SessionMode};
use hydra_server::domain::task_status::Status;
use hydra_server::domain::users::{User, Username};
use hydra_server::store::Store;
use hydra_server::store::sqlite_store::{self, SqliteStore};
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
    let (alice_session_id, alice_token) =
        seed_agent_session_and_token(store.as_ref(), &agent_name, &alice).await?;
    let (bob_session_id, bob_token) =
        seed_agent_session_and_token(store.as_ref(), &agent_name, &bob).await?;
    assert_ne!(
        alice_session_id, bob_session_id,
        "each session must have a distinct SessionId"
    );

    // The two tokens carry the same actor name (`agents/swe`) but
    // different originating creators — this is the exact shape that
    // pre-fix collapsed into a single creator at the actor table.
    let alice_row = store
        .get_auth_token_by_hash(&Actor::hash_auth_token(&alice_token))
        .await?
        .context("alice token must be retrievable by hash")?;
    let bob_row = store
        .get_auth_token_by_hash(&Actor::hash_auth_token(&bob_token))
        .await?
        .context("bob token must be retrievable by hash")?;

    assert_eq!(alice_row.actor_name, format!("agents/{agent_name}"));
    assert_eq!(bob_row.actor_name, format!("agents/{agent_name}"));

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

    // Drive an `add_patch` for each user via the runtime `Actor` that
    // `routes/auth.rs::require_auth` would build from the matched row,
    // and assert the persisted `patch.creator` is the originating user.
    let repo_name = seed_repository(store.as_ref()).await?;

    let alice_patch_id =
        add_patch_as(store.as_ref(), &alice_row, &repo_name, "alice patch").await?;
    let bob_patch_id = add_patch_as(store.as_ref(), &bob_row, &repo_name, "bob patch").await?;

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
/// shared `agents/<name>` actor, then mint an auth token bound to that
/// session — mirroring what `AppState::create_actor_for_job` does, but
/// without dragging in the rest of the app surface.
async fn seed_agent_session_and_token(
    store: &dyn Store,
    agent_name: &AgentName,
    creator: &Username,
) -> Result<(hydra_common::SessionId, String)> {
    let session = Session::new(
        creator.clone(),
        None,
        None,
        AgentConfig::new(Some(agent_name.clone()), None, None, None),
        MountSpec::default(),
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

    let actor_id = ActorId::Agent(agent_name.clone());
    let (_actor, auth_token) =
        Actor::new_from_actor_id(actor_id, creator.clone(), Some(session_id.clone()));

    let actor_name = format!("agents/{agent_name}");
    let raw_token = auth_token
        .split_once(':')
        .map(|(_, raw)| raw)
        .context("auth_token must be '<actor_name>:<raw>'")?;
    let token_hash = Actor::hash_auth_token(raw_token);
    store
        .add_auth_token(&actor_name, &token_hash, Some(&session_id), creator)
        .await?;

    Ok((session_id, raw_token.to_string()))
}

async fn seed_repository(store: &dyn Store) -> Result<RepoName> {
    let name = RepoName::new("dourolabs", "hydra").unwrap();
    let repo = Repository::new(
        "https://github.com/dourolabs/hydra".to_string(),
        Some("main".to_string()),
    );
    store
        .add_repository(name.clone(), repo, &ActorRef::test())
        .await?;
    Ok(name)
}

async fn add_patch_as(
    store: &dyn Store,
    auth_row: &hydra_server::store::AuthTokenRow,
    repo_name: &RepoName,
    title: &str,
) -> Result<hydra_common::PatchId> {
    // Mirrors `routes/auth.rs::require_auth`: build the runtime `Actor`
    // straight from the matched `auth_tokens` row. The auth middleware
    // is the load-bearing step — pre-fix, this would have produced an
    // `Actor` whose `creator` was the shared agent row's creator
    // regardless of which session's token came in.
    let actor_id =
        Actor::parse_name(&auth_row.actor_name).context("parse actor_name into ActorId")?;
    let actor = Actor {
        actor_id,
        creator: auth_row.creator.clone(),
        session_id: auth_row.session_id.clone(),
    };

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
    let (patch_id, _) = store.add_patch(patch, &ActorRef::from(&actor)).await?;
    Ok(patch_id)
}
