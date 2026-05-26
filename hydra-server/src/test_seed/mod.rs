//! Deterministic seed for the migration-baseline fixture.
//!
//! Used by the `seed-migration-fixture` binary
//! (`hydra-server/src/bin/seed-migration-fixture/main.rs`) to populate a
//! fresh `metis` schema with a small catalogue of rows covering the shapes
//! downstream migrations care about: issues with each assignee shape,
//! patches with each review-author shape, tasks of each `SessionMode`
//! variant, kebab-case `refers-to` relations, `conversation_events_v2`
//! rows, and legacy `auth_tokens`. See
//! `/designs/pre-prod-deploy-test-plan.md` §5.
//!
//! The output is byte-stable across runs against fresh DBs: every row has
//! an explicit `created_at` / `updated_at`, every id is a literal string,
//! and rows are inserted in a fixed order so that `pg_dump`'s heap-order
//! output matches between runs.

use anyhow::{Context, Result};
use sqlx::PgPool;

/// Fixed timestamp used for every `created_at` / `updated_at` written by
/// the seed. Picking a literal keeps the `pg_dump` diff stable across runs
/// regardless of when the regen tool was invoked.
const SEED_TS: &str = "2026-01-01 00:00:00+00";

/// Populate the `metis` schema with the baseline catalogue.
///
/// The pool must point at a fresh DB on which `MIGRATOR.run` has already
/// applied every migration on the current checkout — the seed inserts
/// rows shaped against the HEAD schema.
pub async fn seed_baseline(pool: &PgPool) -> Result<()> {
    let mut tx = pool.begin().await.context("begin seed transaction")?;

    // --- users_v2 --------------------------------------------------------
    sqlx::query(
        "INSERT INTO metis.users_v2 \
         (id, version_number, username, github_user_id, deleted, actor, created_at, updated_at) \
         VALUES ('u-alice', 1, 'alice', NULL, FALSE, NULL, $1::timestamptz, $1::timestamptz)",
    )
    .bind(SEED_TS)
    .execute(&mut *tx)
    .await
    .context("insert user")?;

    // --- actors_v2 -------------------------------------------------------
    sqlx::query(
        "INSERT INTO metis.actors_v2 \
         (id, version_number, auth_token_hash, auth_token_salt, actor_id, creator, actor, created_at, updated_at) \
         VALUES ('u-alice', 1, 'hash-alice', 'salt-alice', \
                 '{\"User\":\"alice\"}'::jsonb, NULL, NULL, $1::timestamptz, $1::timestamptz)",
    )
    .bind(SEED_TS)
    .execute(&mut *tx)
    .await
    .context("insert actor")?;

    // --- agents ---------------------------------------------------------
    sqlx::query(
        "INSERT INTO metis.agents \
         (name, prompt_path, max_tries, max_simultaneous, is_assignment_agent, deleted, \
          secrets, mcp_config_path, is_default_conversation_agent, created_at, updated_at) \
         VALUES ('reviewer', 'prompts/reviewer.md', 3, 1, FALSE, FALSE, \
                 '[]'::jsonb, NULL, FALSE, $1::timestamptz, $1::timestamptz)",
    )
    .bind(SEED_TS)
    .execute(&mut *tx)
    .await
    .context("insert agent")?;

    // --- repositories_v2 -------------------------------------------------
    sqlx::query(
        "INSERT INTO metis.repositories_v2 \
         (id, version_number, remote_url, default_branch, default_image, \
          deleted, merge_policy, actor, created_at, updated_at) \
         VALUES ('dourolabs/hydra', 1, 'https://github.com/dourolabs/hydra', 'main', \
                 NULL, FALSE, NULL, NULL, $1::timestamptz, $1::timestamptz)",
    )
    .bind(SEED_TS)
    .execute(&mut *tx)
    .await
    .context("insert repo")?;

    // --- issues_v2: one row per assignee shape ---------------------------
    // (1) bare-username assignee -> typed user Principal
    // (2) prefixed agent assignee -> typed agent Principal
    // (3) slash-bearing external assignee (legacy path form) -> typed External
    // (4) NULL assignee
    // (5) already-typed Principal stored explicitly (no legacy `assignee` text)
    let issues: &[(&str, Option<&str>, Option<&str>)] = &[
        (
            "i-bare-user",
            Some("alice"),
            Some(r#"{"kind":"user","name":"alice"}"#),
        ),
        (
            "i-prefixed-agent",
            Some("agents/reviewer"),
            Some(r#"{"kind":"agent","name":"reviewer"}"#),
        ),
        (
            "i-external",
            Some("external/linear/HYDRA-123"),
            Some(r#"{"kind":"external","system":"linear","username":"HYDRA-123"}"#),
        ),
        ("i-null-assignee", None, None),
        (
            "i-typed-only",
            None,
            Some(r#"{"kind":"user","name":"alice"}"#),
        ),
    ];
    for (id, assignee, principal) in issues {
        sqlx::query(
            "INSERT INTO metis.issues_v2 \
             (id, version_number, issue_type, title, description, creator, progress, status, \
              assignee, assignee_principal, job_settings, deleted, actor, form, form_response, \
              feedback, created_at, updated_at) \
             VALUES ($1, 1, 'task', $1, $1, 'alice', '', 'open', \
                     $2, $3::jsonb, '{}'::jsonb, FALSE, NULL, NULL, NULL, \
                     NULL, $4::timestamptz, $4::timestamptz)",
        )
        .bind(id)
        .bind(*assignee)
        .bind(*principal)
        .bind(SEED_TS)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("insert issue {id}"))?;
    }

    // --- patches_v2: one row per review-author shape ---------------------
    let patches: &[(&str, &str)] = &[
        (
            "p-bare-author",
            // Legacy: bare-string author — the on-disk shape PR-1's
            // migration_roundtrip test wants to see flowing through future
            // Principal rewrites.
            r#"[{"author":"alice","contents":"lgtm","is_approved":true,"submitted_at":"2026-01-01T00:00:00Z"}]"#,
        ),
        (
            "p-typed-user-author",
            r#"[{"author":{"kind":"user","name":"alice"},"contents":"lgtm","is_approved":true,"submitted_at":"2026-01-01T00:00:00Z"}]"#,
        ),
        (
            "p-typed-agent-author",
            r#"[{"author":{"kind":"agent","name":"reviewer"},"contents":"nit","is_approved":false,"submitted_at":"2026-01-01T00:00:00Z"}]"#,
        ),
    ];
    for (id, reviews) in patches {
        sqlx::query(
            "INSERT INTO metis.patches_v2 \
             (id, version_number, title, description, diff, status, is_automatic_backup, \
              reviews, service_repo_name, github, deleted, branch_name, \
              commit_range, creator, base_branch, actor, created_at, updated_at) \
             VALUES ($1, 1, $1, $1, '', 'open', FALSE, \
                     $2::jsonb, 'dourolabs/hydra', NULL, FALSE, $1, \
                     NULL, 'alice', 'main', NULL, $3::timestamptz, $3::timestamptz)",
        )
        .bind(id)
        .bind(*reviews)
        .bind(SEED_TS)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("insert patch {id}"))?;
    }

    // --- conversations_v2 + conversation_events_v2 ------------------------
    sqlx::query(
        "INSERT INTO metis.conversations_v2 \
         (id, version_number, title, agent_name, status, creator, \
          deleted, actor, session_settings, created_at, updated_at) \
         VALUES ('c-baseline', 1, 'baseline-convo', 'reviewer', 'active', 'alice', \
                 FALSE, NULL, '{}'::jsonb, $1::timestamptz, $1::timestamptz)",
    )
    .bind(SEED_TS)
    .execute(&mut *tx)
    .await
    .context("insert conversation")?;

    let events: &[(&str, &str)] = &[
        ("user_message", r#"{"role":"user","content":"hello"}"#),
        (
            "assistant_message",
            r#"{"role":"assistant","content":"hi"}"#,
        ),
        ("system_event", r#"{"note":"agent attached"}"#),
    ];
    for (i, (event_type, data)) in events.iter().enumerate() {
        sqlx::query(
            "INSERT INTO metis.conversation_events_v2 \
             (conversation_id, version_number, event_type, event_data, actor, created_at) \
             VALUES ('c-baseline', $1, $2, $3::jsonb, NULL, $4::timestamptz)",
        )
        .bind(i as i64 + 1)
        .bind(*event_type)
        .bind(*data)
        .bind(SEED_TS)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("insert conversation_event {event_type}"))?;
    }

    // --- tasks_v2: one row per SessionMode variant -----------------------
    let headless_mount = r#"{"working_dir":"repo","mounts":[{"type":"bundle","target":"repo","bundle":"none","session_id":"s-headless"}]}"#;
    let interactive_mount = r#"{"working_dir":"repo","mounts":[{"type":"bundle","target":"repo","bundle":"none","session_id":"s-interactive"}]}"#;
    let agent_cfg =
        r#"{"agent_name":"reviewer","model":null,"system_prompt":null,"mcp_config":null}"#;

    let tasks: &[(&str, Option<&str>, &str, &str)] = &[
        (
            "s-headless",
            None,
            headless_mount,
            r#"{"type":"headless","prompt":"do thing"}"#,
        ),
        (
            "s-interactive",
            Some("c-baseline"),
            interactive_mount,
            r#"{"type":"interactive","conversation_id":"c-baseline","idle_timeout_secs":null,"conversation_resume_from":null}"#,
        ),
    ];
    for (id, conv_id, mount, mode) in tasks {
        sqlx::query(
            "INSERT INTO metis.tasks_v2 \
             (id, version_number, spawned_from, creator, image, env_vars, cpu_limit, memory_limit, \
              status, last_message, error, deleted, actor, secrets, creation_time, start_time, \
              end_time, conversation_id, usage, mount_spec, agent_config, mode, resumed_from, \
              created_at, updated_at) \
             VALUES ($1, 1, NULL, 'alice', NULL, '{}'::jsonb, NULL, NULL, \
                     'complete', NULL, NULL, FALSE, NULL, NULL, $5::timestamptz, NULL, \
                     NULL, $2, NULL, $3::jsonb, $4::jsonb, $6::jsonb, NULL, \
                     $5::timestamptz, $5::timestamptz)",
        )
        .bind(id)
        .bind(*conv_id)
        .bind(*mount)
        .bind(agent_cfg)
        .bind(SEED_TS)
        .bind(*mode)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("insert task {id}"))?;
    }

    // --- documents_v2 ----------------------------------------------------
    sqlx::query(
        "INSERT INTO metis.documents_v2 \
         (id, version_number, title, body_markdown, path, deleted, actor, \
          created_at, updated_at) \
         VALUES ('d-baseline', 1, 'baseline-doc', '# baseline', '/notes/baseline.md', \
                 FALSE, NULL, $1::timestamptz, $1::timestamptz)",
    )
    .bind(SEED_TS)
    .execute(&mut *tx)
    .await
    .context("insert document")?;

    // --- object_relationships: kebab-case rel_types ----------------------
    let rels: &[(&str, &str, &str, &str, &str)] = &[
        (
            "i-bare-user",
            "issue",
            "i-prefixed-agent",
            "issue",
            "child-of",
        ),
        (
            "i-bare-user",
            "issue",
            "p-bare-author",
            "patch",
            "has-patch",
        ),
        (
            "i-bare-user",
            "issue",
            "d-baseline",
            "document",
            "refers-to",
        ),
    ];
    for (src, src_kind, tgt, tgt_kind, rel) in rels {
        sqlx::query(
            "INSERT INTO metis.object_relationships \
             (source_id, source_kind, target_id, target_kind, rel_type, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6::timestamptz)",
        )
        .bind(*src)
        .bind(*src_kind)
        .bind(*tgt)
        .bind(*tgt_kind)
        .bind(*rel)
        .bind(SEED_TS)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("insert relationship {src}->{tgt}/{rel}"))?;
    }

    // --- auth_tokens: legacy (session_id IS NULL) + session-bound --------
    sqlx::query(
        "INSERT INTO metis.auth_tokens \
         (actor_name, token_hash, session_id, is_revoked, created_at) \
         VALUES ('u-alice', 'legacy-hash', NULL, FALSE, $1::timestamptz), \
                ('u-alice', 'session-hash', 's-interactive', FALSE, $1::timestamptz)",
    )
    .bind(SEED_TS)
    .execute(&mut *tx)
    .await
    .context("insert auth_tokens")?;

    tx.commit().await.context("commit seed transaction")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgPoolOptions;
    use sqlx::{Executor, PgPool};

    /// Connects to the per-test postgres pool indicated by `DATABASE_URL`,
    /// drops + recreates the `metis` schema, runs migrations to HEAD, and
    /// returns the prepared pool.
    async fn prepare_pool(dsn: &str) -> Result<PgPool> {
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(dsn)
            .await
            .context("connect to seed-determinism test DB")?;
        pool.execute("DROP SCHEMA IF EXISTS metis CASCADE; CREATE SCHEMA metis;")
            .await
            .context("reset metis schema")?;
        crate::ee::store::postgres_v2::run_migrations(&pool).await?;
        Ok(pool)
    }

    /// Capture `pg_dump --data-only --inserts --column-inserts --schema=metis`
    /// against the given DSN.
    fn pg_dump_snapshot(dsn: &str) -> Result<String> {
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
        String::from_utf8(out.stdout).context("pg_dump output not utf-8")
    }

    /// Run `seed_baseline` twice against freshly-reset databases and assert
    /// the `pg_dump` snapshots are byte-identical.
    ///
    /// Requires `DATABASE_URL` to point at a postgres DSN the test process
    /// can mutate freely (it will drop the `metis` schema). Ignored by
    /// default so the broader unit test suite stays hermetic.
    #[tokio::test]
    #[ignore = "requires DATABASE_URL pointing at a writeable postgres + pg_dump on PATH"]
    async fn seed_baseline_is_deterministic() -> Result<()> {
        let dsn = std::env::var("DATABASE_URL")
            .context("DATABASE_URL must be set for the determinism test")?;

        let pool_a = prepare_pool(&dsn).await?;
        seed_baseline(&pool_a).await?;
        let snap_a = pg_dump_snapshot(&dsn)?;
        pool_a.close().await;

        let pool_b = prepare_pool(&dsn).await?;
        seed_baseline(&pool_b).await?;
        let snap_b = pg_dump_snapshot(&dsn)?;
        pool_b.close().await;

        assert_eq!(
            snap_a, snap_b,
            "seed_baseline produced non-deterministic output"
        );
        Ok(())
    }
}
