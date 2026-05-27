//! Deterministic seed for the migration-baseline fixture.
//!
//! Drives `pg_dump` to byte-stable output by:
//!   1. Routing every row through [`PostgresStoreV2`] (no raw `INSERT`
//!      statements in this module — the spec for `[[i-nytedgut]]` is
//!      explicit about that) so future schema additions are picked up
//!      via the `Store` trait instead of by editing this file.
//!   2. Driving IDs and content from a seeded [`StdRng`] so two runs
//!      against a fresh DB produce the same rows.
//!   3. Pinning `created_at` / `updated_at` / `creation_time` post-insert
//!      via [`PostgresStoreV2::seed_pin_timestamps`], which bypasses the
//!      `metis.touch_updated_at` trigger by running the UPDATEs with
//!      `session_replication_role = 'replica'`.
//!
//! Coverage matches what the prior raw-SQL seed exercised (issues with
//! every assignee Principal shape, patches with every Review.author
//! Principal shape, tasks for every `SessionMode` variant, kebab
//! `refers-to` object_relationships, conversation events, and both
//! legacy NULL-`session_id` and session-bound `auth_tokens`) and adds a
//! [`SeedConfig`] scale knob so callers can stress migration tooling
//! with a larger catalogue from the same seed.

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDateTime, Utc};
use rand::{Rng, SeedableRng, rngs::StdRng};

use hydra_common::api::v1::agents::AgentName;
use hydra_common::api::v1::sessions::{Bundle, MountItem, MountSpec, RelativePath};
use hydra_common::api::v1::users::Username as ApiUsername;
use hydra_common::principal::{ExternalSystem, Principal};
use hydra_common::repositories::Repository;
use hydra_common::{
    ActorRef, ConversationId, DocumentId, HydraId, IssueId, PatchId, RepoName, SessionId,
};

use hydra_server::domain::actors::{Actor, ActorId};
use hydra_server::domain::agents::Agent;
use hydra_server::domain::conversations::{Conversation, ConversationEvent, ConversationStatus};
use hydra_server::domain::documents::Document;
use hydra_server::domain::issues::{Issue, IssueStatus, IssueType};
use hydra_server::domain::patches::{Patch, PatchStatus, Review};
use hydra_server::domain::sessions::{AgentConfig, Session, SessionMode};
use hydra_server::domain::task_status::Status;
use hydra_server::domain::users::{User, Username as DomainUsername};
use hydra_server::store::Store;
use hydra_server::store::postgres_v2::PostgresStoreV2;

/// Fixed timestamp every persisted row is pinned to after inserts.
/// Picking a literal keeps the `pg_dump` diff stable across runs
/// regardless of when the regen tool was invoked.
const SEED_TS_RAW: &str = "2026-01-01T00:00:00Z";

/// Fixed PRNG seed. Any change here invalidates the byte-equality of
/// every subsequent baseline regen; bump only with intent.
const SEED_PRNG: u64 = 0x6D69_6772_6174_696E; // "migratin"

/// ID suffix length used by the seed. 10 chars of `[a-z]` is 26^10 ≈ 1.4e14
/// distinct ids — plenty of headroom against any plausible `--scale` knob.
const ID_SUFFIX_LEN: usize = 10;

/// Per-entity row counts. Each field is a multiplier on the corresponding
/// per-shape minimum that the prior raw-SQL seed covered, so `SeedConfig::for_scale(1)`
/// produces baseline coverage (one row per assignee shape, etc.) and larger
/// scales fan out for migration-stress testing without changing the kinds.
#[derive(Debug, Clone, Copy)]
pub struct SeedConfig {
    /// Replicas of each issue-assignee shape (5 shapes total).
    pub issues_per_shape: usize,
    /// Replicas of each Review.author shape (3 shapes total).
    pub patches_per_shape: usize,
    /// Documents to insert.
    pub documents: usize,
    /// Conversations to insert.
    pub conversations: usize,
    /// Conversation events appended to each conversation.
    pub events_per_conversation: usize,
    /// Headless sessions to insert. (Interactive sessions are 1:1 with
    /// conversations so they need not be scaled separately.)
    pub headless_sessions: usize,
}

impl SeedConfig {
    /// `for_scale(1)` matches the original seed's coverage; values >1
    /// fan everything out by that factor.
    pub fn for_scale(scale: usize) -> Self {
        let n = scale.max(1);
        Self {
            issues_per_shape: n,
            patches_per_shape: n,
            documents: n,
            conversations: n,
            events_per_conversation: 3 * n,
            headless_sessions: n,
        }
    }
}

impl Default for SeedConfig {
    fn default() -> Self {
        Self::for_scale(1)
    }
}

/// Populate the `metis` schema with the baseline catalogue.
///
/// The pool wrapped by `store` must point at a fresh DB on which
/// `run_migrations` has already applied every migration on the current
/// checkout — the seed inserts rows shaped against the HEAD schema.
pub async fn seed_baseline(store: &PostgresStoreV2, config: SeedConfig) -> Result<()> {
    let mut rng = StdRng::seed_from_u64(SEED_PRNG);
    let ts: DateTime<Utc> = NaiveDateTime::parse_from_str(SEED_TS_RAW, "%Y-%m-%dT%H:%M:%SZ")
        .context("parse SEED_TS_RAW")?
        .and_utc();

    // --- users + matching actors --------------------------------------
    // Two "alice"-flavored seed users gives us the source-of-truth User row
    // plus a stable peer that other assignment shapes (e.g. another agent's
    // creator) can point at. Both are created via the standard Store
    // trait paths.
    let seed_actor = ActorRef::System {
        worker_name: "seed-migration-fixture".to_string(),
        on_behalf_of: None,
    };

    let user_alice_name = DomainUsername::from("alice");
    store
        .add_user(User::new(user_alice_name.clone(), None, false), &seed_actor)
        .await
        .context("seed user alice")?;
    let user_bob_name = DomainUsername::from("bob");
    store
        .add_user(User::new(user_bob_name.clone(), None, false), &seed_actor)
        .await
        .context("seed user bob")?;

    let actor_alice = Actor {
        actor_id: ActorId::User(ApiUsername::try_new("alice").expect("valid username")),
        creator: user_alice_name.clone(),
        session_id: None,
    };
    store
        .add_actor(actor_alice, &seed_actor)
        .await
        .context("seed actor alice")?;

    // --- agent --------------------------------------------------------
    let reviewer = Agent::new(
        "reviewer".to_string(),
        "prompts/reviewer.md".to_string(),
        None,
        3,
        1,
        false,
        false,
        Vec::new(),
    );
    store
        .add_agent(reviewer)
        .await
        .context("seed reviewer agent")?;

    // --- repository ---------------------------------------------------
    let repo_name = RepoName::new("dourolabs", "hydra").expect("valid repo name");
    let repo = Repository::new(
        "https://github.com/dourolabs/hydra".to_string(),
        Some("main".to_string()),
        None,
    );
    store
        .add_repository(repo_name.clone(), repo, &seed_actor)
        .await
        .context("seed repository")?;

    // --- issues: one (× scale) per assignee Principal shape -----------
    //
    // Acceptance criterion 2 says "every assignee shape" — we drive the
    // five Principal-shaped buckets via a closure so adding another shape
    // is a one-line change and the seed PRNG is the only source of
    // variation between rows of the same shape.
    let alice_principal = Principal::user(ApiUsername::try_new("alice").unwrap());
    let reviewer_principal = Principal::agent(AgentName::try_new("reviewer").unwrap());
    let external_principal = Principal::external(
        ExternalSystem::try_new("linear").expect("valid external system"),
        "HYDRA-123",
    );
    let assignee_shapes: [Option<Principal>; 5] = [
        Some(alice_principal.clone()),
        Some(reviewer_principal.clone()),
        Some(external_principal),
        None,
        Some(alice_principal.clone()),
    ];

    let mut issue_ids: Vec<IssueId> = Vec::new();
    for shape in &assignee_shapes {
        for _ in 0..config.issues_per_shape {
            let id = mint_id(&mut rng, IssueId::prefix(), |s| {
                IssueId::try_from(s).expect("valid IssueId")
            });
            let issue = Issue::new(
                IssueType::Task,
                random_phrase(&mut rng, "issue"),
                random_phrase(&mut rng, "described"),
                user_alice_name.clone(),
                String::new(),
                IssueStatus::Open,
                shape.clone(),
                None,
                Vec::new(),
                Vec::new(),
                None,
                None,
                None,
            );
            store
                .seed_insert_issue(&id, &issue, &seed_actor)
                .await
                .with_context(|| format!("seed issue {id}"))?;
            issue_ids.push(id);
        }
    }

    // --- patches: one (× scale) per Review.author Principal shape -----
    let review_authors: [Principal; 3] = [
        alice_principal.clone(),
        reviewer_principal.clone(),
        Principal::user(ApiUsername::try_new("bob").unwrap()),
    ];

    let mut patch_ids: Vec<PatchId> = Vec::new();
    for author in &review_authors {
        for _ in 0..config.patches_per_shape {
            let id = mint_id(&mut rng, PatchId::prefix(), |s| {
                PatchId::try_from(s).expect("valid PatchId")
            });
            let review = Review::new(
                "looks good".to_string(),
                rng.r#gen::<bool>(),
                author.clone(),
                Some(ts),
            );
            let patch = Patch::new(
                random_phrase(&mut rng, "patch"),
                random_phrase(&mut rng, "patch desc"),
                String::new(),
                PatchStatus::Open,
                false,
                user_alice_name.clone(),
                vec![review],
                repo_name.clone(),
                None,
                Some(format!("seed/{}", id.as_ref())),
                None,
                Some("main".to_string()),
            );
            store
                .seed_insert_patch(&id, &patch, &seed_actor)
                .await
                .with_context(|| format!("seed patch {id}"))?;
            patch_ids.push(id);
        }
    }

    // --- documents ----------------------------------------------------
    let mut document_ids: Vec<DocumentId> = Vec::new();
    for i in 0..config.documents {
        let id = mint_id(&mut rng, DocumentId::prefix(), |s| {
            DocumentId::try_from(s).expect("valid DocumentId")
        });
        let path = format!("/notes/seed-{i:04}.md").parse().ok();
        let doc = Document {
            title: random_phrase(&mut rng, "doc"),
            body_markdown: format!("# seed-{i}\n\n{}\n", random_phrase(&mut rng, "body")),
            path,
            deleted: false,
        };
        store
            .seed_insert_document(&id, &doc, &seed_actor)
            .await
            .with_context(|| format!("seed document {id}"))?;
        document_ids.push(id);
    }

    // --- conversations + events ---------------------------------------
    let mut conversation_ids: Vec<ConversationId> = Vec::new();
    for _ in 0..config.conversations {
        let id = mint_id(&mut rng, ConversationId::prefix(), |s| {
            ConversationId::try_from(s).expect("valid ConversationId")
        });
        let conv = Conversation {
            title: Some(random_phrase(&mut rng, "convo")),
            agent_name: Some(AgentName::try_new("reviewer").unwrap()),
            status: ConversationStatus::Active,
            creator: user_alice_name.clone(),
            session_settings: Default::default(),
            deleted: false,
        };
        store
            .seed_insert_conversation(&id, &conv, &seed_actor)
            .await
            .with_context(|| format!("seed conversation {id}"))?;
        for _ in 0..config.events_per_conversation {
            let event = ConversationEvent::Suspending {
                reason: random_phrase(&mut rng, "suspend"),
                timestamp: ts,
            };
            store
                .append_conversation_event(&id, event, &seed_actor)
                .await
                .with_context(|| format!("seed conversation event for {id}"))?;
        }
        conversation_ids.push(id);
    }

    // --- sessions: one per SessionMode variant + scaled headless count.
    let mut session_ids: Vec<SessionId> = Vec::new();

    // Interactive sessions are 1:1 with conversations so the Conversation
    // FK remains satisfied for every row.
    for conv_id in &conversation_ids {
        let id = mint_id(&mut rng, SessionId::prefix(), |s| {
            SessionId::try_from(s).expect("valid SessionId")
        });
        let mount_spec = MountSpec::new(
            RelativePath::new("repo").unwrap(),
            vec![MountItem::Bundle {
                target: RelativePath::new("repo").unwrap(),
                bundle: Bundle::None,
            }],
        );
        let session = Session::new(
            user_alice_name.clone(),
            None,
            None,
            AgentConfig::new(
                Some(AgentName::try_new("reviewer").unwrap()),
                None,
                None,
                None,
            ),
            mount_spec,
            None,
            std::collections::HashMap::new(),
            None,
            None,
            None,
            SessionMode::Interactive {
                conversation_id: conv_id.clone(),
                idle_timeout_secs: None,
                conversation_resume_from: None,
            },
            Status::Complete,
            None,
            None,
        );
        store
            .seed_insert_session(&id, session, ts, &seed_actor)
            .await
            .with_context(|| format!("seed interactive session {id}"))?;
        session_ids.push(id);
    }

    for i in 0..config.headless_sessions {
        let id = mint_id(&mut rng, SessionId::prefix(), |s| {
            SessionId::try_from(s).expect("valid SessionId")
        });
        let mount_spec = MountSpec::new(
            RelativePath::new("repo").unwrap(),
            vec![MountItem::Bundle {
                target: RelativePath::new("repo").unwrap(),
                bundle: Bundle::None,
            }],
        );
        let session = Session::new(
            user_alice_name.clone(),
            None,
            None,
            AgentConfig::default(),
            mount_spec,
            None,
            std::collections::HashMap::new(),
            None,
            None,
            None,
            SessionMode::Headless {
                prompt: format!("headless-{i}: {}", random_phrase(&mut rng, "do")),
            },
            Status::Complete,
            None,
            None,
        );
        store
            .seed_insert_session(&id, session, ts, &seed_actor)
            .await
            .with_context(|| format!("seed headless session {id}"))?;
        session_ids.push(id);
    }

    // --- object_relationships (kebab-case rel_types) ------------------
    //
    // We exercise child-of (issue→issue), has-patch (issue→patch), and
    // refers-to (issue→document). `add_relationship` infers the kind from
    // the id prefix so we only need the typed ids on either end.
    if issue_ids.len() >= 2 {
        let (head, tail) = issue_ids.split_first().unwrap();
        let target = &tail[0];
        store
            .add_relationship(
                &HydraId::from(head.clone()),
                &HydraId::from(target.clone()),
                hydra_server::store::RelationshipType::ChildOf,
            )
            .await
            .context("seed child-of relationship")?;
    }
    if let (Some(issue), Some(patch)) = (issue_ids.first(), patch_ids.first()) {
        store
            .add_relationship(
                &HydraId::from(issue.clone()),
                &HydraId::from(patch.clone()),
                hydra_server::store::RelationshipType::HasPatch,
            )
            .await
            .context("seed has-patch relationship")?;
    }
    if let (Some(issue), Some(doc)) = (issue_ids.first(), document_ids.first()) {
        store
            .add_relationship(
                &HydraId::from(issue.clone()),
                &HydraId::from(doc.clone()),
                hydra_server::store::RelationshipType::RefersTo,
            )
            .await
            .context("seed refers-to relationship")?;
    }

    // --- auth_tokens (legacy NULL session_id + session-bound) ---------
    //
    // The actor name we wrote above is `ActorId::User("alice").to_string()`
    // which renders as `users/alice` (see `actor_id::Display`).
    let actor_name = "users/alice";
    store
        .add_auth_token(actor_name, &random_hash(&mut rng, "legacy"), None)
        .await
        .context("seed legacy auth_token")?;
    if let Some(session_id) = session_ids.first() {
        store
            .add_auth_token(
                actor_name,
                &random_hash(&mut rng, "session"),
                Some(session_id),
            )
            .await
            .context("seed session-bound auth_token")?;
    }

    // --- pin timestamps so pg_dump is byte-stable ---------------------
    //
    // Strategy (b) from `[[i-nytedgut]]`'s Notes/risks: post-insert
    // UPDATEs with the trigger bypassed. Done via
    // `PostgresStoreV2::seed_pin_timestamps`, which scopes the
    // `session_replication_role = 'replica'` to a single transaction.
    store
        .seed_pin_timestamps(ts)
        .await
        .context("pin seed timestamps")?;

    // Silence unused-warning when `bob` happens not to be referenced by
    // a sampled shape at scale=1 in the future.
    let _ = user_bob_name;

    Ok(())
}

/// Mint a typed Hydra id from the seeded PRNG. The closure converts the
/// final `<prefix><suffix>` string into the caller's typed wrapper so the
/// validation impl lives next to the type definition (in `hydra-common`)
/// rather than being duplicated here.
fn mint_id<R: Rng, T, F: FnOnce(String) -> T>(rng: &mut R, prefix: &str, ctor: F) -> T {
    let mut id = String::with_capacity(prefix.len() + ID_SUFFIX_LEN);
    id.push_str(prefix);
    for _ in 0..ID_SUFFIX_LEN {
        let offset: u8 = rng.gen_range(0..26);
        id.push((b'a' + offset) as char);
    }
    ctor(id)
}

/// Deterministic short phrase made of seeded random tokens. Used for
/// title / description / prompt fields where the actual contents do not
/// influence schema-migration behavior — they just need to differ between
/// rows so the seed output isn't degenerate.
fn random_phrase<R: Rng>(rng: &mut R, tag: &str) -> String {
    let n = 4 + rng.gen_range(0..6);
    let mut s = String::with_capacity(tag.len() + 1 + n);
    s.push_str(tag);
    s.push('-');
    for _ in 0..n {
        let offset: u8 = rng.gen_range(0..26);
        s.push((b'a' + offset) as char);
    }
    s
}

/// Deterministic "hash" stand-in for `auth_tokens.token_hash`. The
/// migration tooling only needs the column non-null; the actual value is
/// stable across runs because the PRNG is seeded.
fn random_hash<R: Rng>(rng: &mut R, tag: &str) -> String {
    let mut s = format!("{tag}-");
    for _ in 0..32 {
        let n: u8 = rng.gen_range(0..16);
        let c = if n < 10 {
            (b'0' + n) as char
        } else {
            (b'a' + (n - 10)) as char
        };
        s.push(c);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;
    use sqlx::postgres::PgPoolOptions;
    use sqlx::{Executor, PgPool};

    /// Connects to the per-test postgres pool indicated by `DATABASE_URL`,
    /// drops + recreates the `metis` schema, runs migrations to HEAD, and
    /// returns the prepared pool.
    ///
    /// Also drops `public._sqlx_migrations` so the next `MIGRATOR.run`
    /// replays from scratch. Without that, the second call within one test
    /// would find every migration "applied" and skip them, leaving the
    /// freshly-recreated `metis` schema empty. Mirrors the equivalent
    /// reset in `hydra-server/tests/migration_roundtrip.rs`.
    async fn prepare_pool(dsn: &str) -> anyhow::Result<PgPool> {
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(dsn)
            .await
            .context("connect to seed-determinism test DB")?;
        pool.execute(
            "DROP SCHEMA IF EXISTS metis CASCADE; \
             CREATE SCHEMA metis; \
             DROP TABLE IF EXISTS public._sqlx_migrations;",
        )
        .await
        .context("reset metis schema and sqlx migration tracking table")?;
        hydra_server::store::postgres_v2::run_migrations(&pool, None).await?;
        Ok(pool)
    }

    /// Capture `pg_dump --data-only --inserts --column-inserts --schema=metis`
    /// against the given DSN.
    fn pg_dump_snapshot(dsn: &str) -> anyhow::Result<String> {
        let out = std::process::Command::new("pg_dump")
            .args([
                "--data-only",
                "--inserts",
                "--column-inserts",
                "--schema=metis",
                dsn,
            ])
            .output()
            .context("invoke pg_dump")?;
        anyhow::ensure!(
            out.status.success(),
            "pg_dump failed (status {:?}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr),
        );
        let raw = String::from_utf8(out.stdout).context("pg_dump output not utf-8")?;
        Ok(strip_pg_dump_restrict_tokens(&raw))
    }

    /// pg_dump 16.14+ emits a fresh random token on each invocation in the
    /// leading `\restrict <token>` and trailing `\unrestrict <token>` psql
    /// meta-commands, which makes byte-wise snapshot comparison flaky. The
    /// payload between those lines is deterministic; drop the wrapper lines
    /// so the determinism check actually compares the data.
    fn strip_pg_dump_restrict_tokens(dump: &str) -> String {
        let mut out = String::with_capacity(dump.len());
        for line in dump.lines() {
            if line.starts_with("\\restrict ") || line.starts_with("\\unrestrict ") {
                continue;
            }
            out.push_str(line);
            out.push('\n');
        }
        out
    }

    /// Run `seed_baseline` twice against freshly-reset databases and
    /// assert the `pg_dump` snapshots are byte-identical.
    ///
    /// Requires `DATABASE_URL` to point at a postgres DSN the test
    /// process can mutate freely (it will drop the `metis` schema).
    /// Ignored by default so the broader unit test suite stays hermetic.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL pointing at a writeable postgres + pg_dump on PATH"]
    async fn seed_baseline_is_deterministic() -> anyhow::Result<()> {
        let dsn = std::env::var("DATABASE_URL")
            .context("DATABASE_URL must be set for the determinism test")?;

        let pool_a = prepare_pool(&dsn).await?;
        let store_a = PostgresStoreV2::new(pool_a.clone());
        seed_baseline(&store_a, SeedConfig::default()).await?;
        let snap_a = pg_dump_snapshot(&dsn)?;
        pool_a.close().await;

        let pool_b = prepare_pool(&dsn).await?;
        let store_b = PostgresStoreV2::new(pool_b.clone());
        seed_baseline(&store_b, SeedConfig::default()).await?;
        let snap_b = pg_dump_snapshot(&dsn)?;
        pool_b.close().await;

        assert_eq!(
            snap_a, snap_b,
            "seed_baseline produced non-deterministic output"
        );
        Ok(())
    }
}
