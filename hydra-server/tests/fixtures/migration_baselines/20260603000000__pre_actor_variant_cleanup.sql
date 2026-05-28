-- baseline-version: 20260603000000
-- Pre-actor-variant-cleanup shapes. INSERTs are valid against the
-- schema state at version `20260603000000` (immediately after
-- `20260603000000_actor_variant_cleanup_anchor.sql`, before the Rust
-- `actor_variant_cleanup` migration interleaves and rewrites the
-- `actor` / `actor_id` JSON blobs).
--
-- IDs use all-alphabetic suffixes so they parse through the typed
-- `IssueId` / `SessionId` newtypes for the §3.3 store-level smoke.

--------------------------------------------------------------------------------
-- A parent issue that the `Issue` actor-rewrite arm resolves through
-- a `tasks_v2.spawned_from` lookup.
--
-- `i-actissone` has exactly one matching tasks_v2 row, with the task's
-- own actor being a User. The cleanup must replace
-- {"Issue":"i-actissone"} with the resolved {"User": {"name": "alice"}}.
--
-- `i-actisstwo` has ZERO matching tasks_v2 rows; the cleanup must
-- NULL the affected `actor` column rather than failing the migration.
--------------------------------------------------------------------------------
INSERT INTO metis.issues_v2 (id, version_number, issue_type, description, creator)
VALUES
    ('i-actissone', 1, 'task', 'parent for Issue-arm lookup', 'alice'),
    ('i-actisstwo', 1, 'task', 'parent for no-match Issue-arm', 'alice');

INSERT INTO metis.tasks_v2 (id, version_number, creator, status, deleted, is_latest,
                            spawned_from, conversation_id,
                            mount_spec, agent_config, mode,
                            actor, creation_time)
VALUES (
    's-spawnone', 1, 'alice', 'complete', FALSE, TRUE,
    'i-actissone', NULL,
    '{"working_dir":"repo","mounts":[]}'::jsonb,
    '{}'::jsonb,
    '{"type":"headless","prompt":"do thing"}'::jsonb,
    '{"Authenticated": {"actor_id": {"User": {"name": "alice"}}}}'::jsonb,
    '2026-05-10T10:00:00Z'
);

--------------------------------------------------------------------------------
-- issues_v2.actor — exercise every pre-cleanup shape that the
-- migration knows how to rewrite, plus a no-op already-typed row.
-- All IDs use all-alphabetic suffixes so they parse through
-- IssueId::from_str for the §3.3 store-level smoke layer.
--------------------------------------------------------------------------------
INSERT INTO metis.issues_v2 (id, version_number, issue_type, description, creator, actor)
VALUES
    -- 1. Username -> User
    ('i-actuname', 1, 'task', 'username actor', 'alice',
     '{"Authenticated":{"actor_id":{"Username":"alice"}}}'::jsonb),
    -- 2. Session -> Adhoc
    ('i-actsess',  1, 'task', 'session actor', 'alice',
     '{"Authenticated":{"actor_id":{"Session":"s-sessone"}}}'::jsonb),
    -- 3. Issue (with matching tasks_v2 row) -> resolved User
    ('i-actiss',   1, 'task', 'issue actor with match', 'alice',
     '{"Authenticated":{"actor_id":{"Issue":"i-actissone"}}}'::jsonb),
    -- 4. Issue (without matching tasks_v2 row) -> NULL
    ('i-actissno', 1, 'task', 'issue actor without match', 'alice',
     '{"Authenticated":{"actor_id":{"Issue":"i-actisstwo"}}}'::jsonb),
    -- 5. Service with valid AgentName -> Agent
    ('i-actsvcok', 1, 'task', 'service-as-agent actor', 'alice',
     '{"Authenticated":{"actor_id":{"Service":"swe"}}}'::jsonb),
    -- 6. Service with invalid AgentName -> NULL
    ('i-actsvcno', 1, 'task', 'service invalid agent name', 'alice',
     '{"Authenticated":{"actor_id":{"Service":"has space"}}}'::jsonb),
    -- 7. Legacy bare string parseable to User
    ('i-actlegu',  1, 'task', 'legacy users/<x>', 'alice',
     '{"Authenticated":{"actor_id":"users/alice"}}'::jsonb),
    -- 8. Legacy bare string parseable to Agent via agents/swe
    ('i-actlega',  1, 'task', 'legacy agents/<x>', 'alice',
     '{"Authenticated":{"actor_id":"agents/swe"}}'::jsonb),
    -- 9. Legacy unparseable bare string -> NULL
    ('i-actlegx',  1, 'task', 'legacy unparseable', 'alice',
     '{"Authenticated":{"actor_id":"definitely not an actor"}}'::jsonb),
    -- 10. Already-typed User -> no-op
    ('i-actuser',  1, 'task', 'already-typed User', 'alice',
     '{"Authenticated":{"actor_id":{"User":{"name":"alice"}}}}'::jsonb),
    -- 11. Multi-key map (Legacy catch-all) -> NULL
    ('i-actmulti', 1, 'task', 'multi-key actor_id', 'alice',
     '{"Authenticated":{"actor_id":{"kind":"user","name":"alice"}}}'::jsonb);

--------------------------------------------------------------------------------
-- actors_v2.actor + actor_id — the bare ActorId column. The cleanup
-- walks BOTH columns on this table. We add one row per shape.
--------------------------------------------------------------------------------
INSERT INTO metis.actors_v2 (id, version_number, creator, actor_id, actor)
VALUES
    ('actu-aname', 1, 'alice',
     '{"Username":"alice"}'::jsonb,
     '{"Authenticated":{"actor_id":{"Username":"alice"}}}'::jsonb),
    ('actu-asvc',  1, 'alice',
     '{"Service":"swe"}'::jsonb,
     '{"Authenticated":{"actor_id":{"Service":"swe"}}}'::jsonb);

--------------------------------------------------------------------------------
-- conversation_events_v2 — actor column carries an ActorRef with a
-- pre-cleanup inner actor_id. Used by both the cleanup AND the events
-- migration (which reads `Session` via the dual-shape reader).
--------------------------------------------------------------------------------
INSERT INTO metis.conversations_v2 (id, version_number, creator)
VALUES ('c-actclean', 1, 'alice');

INSERT INTO metis.conversation_events_v2
    (conversation_id, version_number, event_type, event_data, actor, created_at)
VALUES
    ('c-actclean', 1, 'suspending',
     '{"type":"suspending","reason":"x","timestamp":"2026-05-10T11:00:00Z"}'::jsonb,
     '{"Authenticated":{"actor_id":{"Session":"s-cesessx"}}}'::jsonb,
     '2026-05-10T11:00:00Z');
