-- baseline-version: 20260608000000
-- Postgres pre-denormalize-creator baseline. INSERTs are valid against
-- the schema state at postgres migration
-- `20260608000000_drop_status_icon.sql`, immediately before
-- `20260609000000_add_creator_to_auth_tokens.sql` denormalizes
-- `metis.auth_tokens.creator` and `20260609010000_drop_actors_v2.sql`
-- removes the `metis.actors_v2` table.
--
-- Scope: per [[i-lnmjbjxk]], the creator-denormalize migration needs
-- migration-framework coverage on both backfill paths plus the
-- subsequent `DROP TABLE actors_v2`:
--   * session-bound token whose `session_id` joins to a `tasks_v2`
--     row with a distinguishing `creator` ('alice').
--   * non-session token whose `actor_name = 'users/bob'`. Asserts
--     the `substring(actor_name FROM length('users/') + 1)` backfill.
--   * an `actors_v2` row that must compile against the pre-drop
--     schema and disappear after `DROP TABLE actors_v2` runs.

-- ------------------------------------------------------------
-- 1. tasks_v2 row for the session-bound token's backfill source.
-- ------------------------------------------------------------
INSERT INTO metis.tasks_v2 (
    id, version_number, creator, status, deleted,
    spawned_from, conversation_id,
    mount_spec, agent_config, mode,
    actor, creation_time
)
VALUES (
    's-alicexx', 1, 'alice', 'complete', FALSE,
    NULL, NULL,
    '{"working_dir":"repo","mounts":[]}'::jsonb,
    '{}'::jsonb,
    '{"type":"headless"}'::jsonb,
    NULL,
    '2026-06-08T10:30:00Z'
);

-- ------------------------------------------------------------
-- 2. auth_tokens rows the new migration must backfill.
-- ------------------------------------------------------------
INSERT INTO metis.auth_tokens (actor_name, token_hash, session_id, is_revoked)
VALUES ('agents/swe', 'hash-session-alice', 's-alicexx', FALSE);

INSERT INTO metis.auth_tokens (actor_name, token_hash, session_id, is_revoked)
VALUES ('users/bob', 'hash-cli-bob', NULL, FALSE);

-- ------------------------------------------------------------
-- 3. actors_v2 row that exercises the `DROP TABLE` step.
-- ------------------------------------------------------------
INSERT INTO metis.actors_v2 (
    id,
    version_number,
    auth_token_hash,
    auth_token_salt,
    actor_id,
    creator,
    actor
)
VALUES (
    'agents/swe',
    1,
    '',
    '',
    '{"Agent":"swe"}'::jsonb,
    'alice',
    NULL
);
