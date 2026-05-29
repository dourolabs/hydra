-- baseline-version: 20260603000000
-- SQLite pre-actor-variant-cleanup baseline. INSERTs are valid against the
-- schema state at sqlite migration `20260603000000_actor_variant_cleanup_anchor.sql`,
-- before the Rust `actor_variant_cleanup` migration interleaves and
-- rewrites the `actor` JSON blobs.
--
-- Scope: the actor-variant-cleanup SQLite arm's `session_events` and
-- `conversation_events` rewrites — the exact code paths surfaced by
-- the `(session_id, version_number) AS __pk` parse-reject bug fixed
-- in p-fcxmstwd. Sister tree to `migration_baselines/` — kept
-- independent so postgres-only fixture changes don't ripple here.
--
-- SQLite differences vs. postgres baseline:
--   * No `metis.` schema prefix.
--   * Boolean columns (`deleted`, `is_latest`) are INTEGERs (0/1) and
--     have no triggers to backfill `is_latest`; we set it explicitly.
--   * `conversation_events` PK is `(id, version_number)` (no `_v2`
--     suffix), where `id` is the conversation id.
--   * `session_events` PK is the synthetic `rowid_seq`, with
--     `(session_id, version_number)` UNIQUE.

-- Parent conversation referenced by `conversation_events` below.
INSERT INTO conversations (id, version_number, creator, is_latest)
VALUES ('c-actclean', 1, 'alice', 1);

-- Parent task referenced by `session_events` below. `mount_spec`,
-- `agent_config`, `mode`, and `creator` are all NOT NULL at this pin.
INSERT INTO tasks_v2 (
    id, version_number, creator, status, deleted, is_latest,
    spawned_from, conversation_id,
    mount_spec, agent_config, mode,
    actor, creation_time
)
VALUES
    ('s-actrowx', 1, 'alice', 'complete', 0, 1,
     NULL, NULL,
     '{"working_dir":"repo","mounts":[]}',
     '{}',
     '{"type":"headless"}',
     '{"Authenticated":{"actor_id":{"User":{"name":"alice"}}}}',
     '2026-05-10T10:30:00Z');

-- session_events: one row per actor shape the cleanup must rewrite,
-- plus an `actor IS NULL` row that must be left alone.
INSERT INTO session_events (session_id, version_number, event_type, event_data, actor)
VALUES
    -- 1. Legacy {"Username":"alice"} -> typed User.
    ('s-actrowx', 1, 'user_message',
     '{"type":"user_message","content":"se username","timestamp":"2026-05-10T10:35:00Z"}',
     '{"Authenticated":{"actor_id":{"Username":"alice"}}}'),
    -- 2. Legacy {"Session":"s-..."} -> typed Adhoc.
    ('s-actrowx', 2, 'user_message',
     '{"type":"user_message","content":"se session","timestamp":"2026-05-10T10:36:00Z"}',
     '{"Authenticated":{"actor_id":{"Session":"s-sessone"}}}'),
    -- 3. Unparseable bare-string Legacy -> External-legacy fallback
    --    preserving the original identifier as the username.
    ('s-actrowx', 3, 'user_message',
     '{"type":"user_message","content":"se bogus","timestamp":"2026-05-10T10:37:00Z"}',
     '{"Authenticated":{"actor_id":"definitely not an actor"}}'),
    -- 4. actor IS NULL -> stays NULL after cleanup.
    ('s-actrowx', 4, 'user_message',
     '{"type":"user_message","content":"se null","timestamp":"2026-05-10T10:38:00Z"}',
     NULL);

-- conversation_events: parallel coverage. Use non-message event types
-- (`suspending` / `closed`) so the events-migration's user_message /
-- assistant_message partitioning does not touch these rows. The
-- cleanup migration walks every conversation_events row regardless of
-- event_type, so this fully exercises the actor-rewrite path.
INSERT INTO conversation_events (id, version_number, event_type, event_data, actor)
VALUES
    -- 1. Legacy {"Session":"s-..."} -> typed Adhoc.
    ('c-actclean', 1, 'suspending',
     '{"type":"suspending","reason":"x","timestamp":"2026-05-10T11:00:00Z"}',
     '{"Authenticated":{"actor_id":{"Session":"s-cesessx"}}}'),
    -- 2. Legacy {"Username":"alice"} -> typed User.
    ('c-actclean', 2, 'closed',
     '{"type":"closed","timestamp":"2026-05-10T11:01:00Z"}',
     '{"Authenticated":{"actor_id":{"Username":"alice"}}}'),
    -- 3. Unparseable bare-string Legacy -> External-legacy fallback.
    ('c-actclean', 3, 'suspending',
     '{"type":"suspending","reason":"y","timestamp":"2026-05-10T11:02:00Z"}',
     '{"Authenticated":{"actor_id":"definitely not an actor"}}'),
    -- 4. actor IS NULL -> stays NULL after cleanup.
    ('c-actclean', 4, 'suspending',
     '{"type":"suspending","reason":"z","timestamp":"2026-05-10T11:03:00Z"}',
     NULL);
