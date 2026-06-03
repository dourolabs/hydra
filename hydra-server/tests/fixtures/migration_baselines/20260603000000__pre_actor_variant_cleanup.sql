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
-- Parent issues that the `Issue` actor-rewrite arm resolves through a
-- `tasks_v2.spawned_from` lookup. Each parent corresponds to a different
-- tie-break edge that `load_issue_to_actor_id` must handle correctly.
--
-- `i-actissone`  — exactly one matching tasks_v2 row, with the task's
--                  own actor being a User. The cleanup must replace
--                  {"Issue":"i-actissone"} with the resolved
--                  {"User": {"name": "alice"}}.
-- `i-actisstwo`  — ZERO matching tasks_v2 rows; cleanup falls back to
--                  External-legacy with the parent issue id preserved
--                  as the username rather than NULLing or failing.
-- `i-actissmany` — TWO non-deleted is_latest=TRUE matching tasks_v2
--                  rows with distinct actors. The lookup map only
--                  inserts when actors.len()==1, so refs fall back to
--                  External-legacy.
-- `i-actissdel`  — One matching task with deleted=TRUE; the loader's
--                  `WHERE deleted = FALSE` skips it -> 0 matches ->
--                  External-legacy fallback.
-- `i-actissold`  — One matching task with is_latest=FALSE; the loader's
--                  `WHERE is_latest = TRUE` skips it -> 0 matches ->
--                  External-legacy fallback.
-- `i-actisschn`  — One matching task whose own actor is a chained
--                  {"Issue":"i-actisstwo"} reference;
--                  `extract_actor_id_from_actor_ref` refuses chained
--                  lookups -> entry omitted from map -> refs fall back
--                  to External-legacy.
--------------------------------------------------------------------------------
INSERT INTO metis.issues_v2 (id, version_number, issue_type, description, creator)
VALUES
    ('i-actissone',  1, 'task', 'parent for Issue-arm lookup',           'alice'),
    ('i-actisstwo',  1, 'task', 'parent for no-match Issue-arm',         'alice'),
    ('i-actissmany', 1, 'task', 'parent for multi-match Issue-arm',      'alice'),
    ('i-actissdel',  1, 'task', 'parent whose only task is deleted',     'alice'),
    ('i-actissold',  1, 'task', 'parent whose only task is not-latest',  'alice'),
    ('i-actisschn',  1, 'task', 'parent whose only task chains Issue',   'alice');

INSERT INTO metis.tasks_v2 (id, version_number, creator, status, deleted, is_latest,
                            spawned_from, conversation_id,
                            mount_spec, agent_config, mode,
                            actor, creation_time)
VALUES
    -- The single-match parent: classic Issue-arm rewrite resolves to User(alice).
    ('s-spawnone',  1, 'alice', 'complete', FALSE, TRUE,
     'i-actissone', NULL,
     '{"working_dir":"repo","mounts":[]}'::jsonb,
     '{}'::jsonb,
     '{"type":"headless"}'::jsonb,
     '{"Authenticated": {"actor_id": {"User": {"name": "alice"}}}}'::jsonb,
     '2026-05-10T10:00:00Z'),
    -- Multi-match parent: two distinct latest non-deleted spawned tasks
    -- -> the lookup map drops `i-actissmany` -> Issue refs fall back to
    -- External-legacy("i-actissmany").
    ('s-spawnmnya', 1, 'alice', 'complete', FALSE, TRUE,
     'i-actissmany', NULL,
     '{"working_dir":"repo","mounts":[]}'::jsonb,
     '{}'::jsonb,
     '{"type":"headless"}'::jsonb,
     '{"Authenticated": {"actor_id": {"User": {"name": "alice"}}}}'::jsonb,
     '2026-05-10T10:00:01Z'),
    ('s-spawnmnyb', 1, 'alice', 'complete', FALSE, TRUE,
     'i-actissmany', NULL,
     '{"working_dir":"repo","mounts":[]}'::jsonb,
     '{}'::jsonb,
     '{"type":"headless"}'::jsonb,
     '{"Authenticated": {"actor_id": {"User": {"name": "bob"}}}}'::jsonb,
     '2026-05-10T10:00:02Z'),
    -- Deleted spawned task: the loader's `WHERE deleted=FALSE` skips it.
    ('s-spawndel',  1, 'alice', 'complete', TRUE, TRUE,
     'i-actissdel', NULL,
     '{"working_dir":"repo","mounts":[]}'::jsonb,
     '{}'::jsonb,
     '{"type":"headless"}'::jsonb,
     '{"Authenticated": {"actor_id": {"User": {"name": "alice"}}}}'::jsonb,
     '2026-05-10T10:00:03Z'),
    -- is_latest=FALSE spawned task: post-INSERT trigger forces is_latest=TRUE
    -- on every fresh row, so we explicitly flip it below.
    ('s-spawnold',  1, 'alice', 'complete', FALSE, TRUE,
     'i-actissold', NULL,
     '{"working_dir":"repo","mounts":[]}'::jsonb,
     '{}'::jsonb,
     '{"type":"headless"}'::jsonb,
     '{"Authenticated": {"actor_id": {"User": {"name": "alice"}}}}'::jsonb,
     '2026-05-10T10:00:04Z'),
    -- Chained-Issue spawned task: actor itself references another Issue.
    -- `extract_actor_id_from_actor_ref` refuses NeedsIssueLookup outcomes,
    -- so `i-actisschn` never makes it into the lookup map.
    ('s-spawnchn',  1, 'alice', 'complete', FALSE, TRUE,
     'i-actisschn', NULL,
     '{"working_dir":"repo","mounts":[]}'::jsonb,
     '{}'::jsonb,
     '{"type":"headless"}'::jsonb,
     '{"Authenticated": {"actor_id": {"Issue": "i-actisstwo"}}}'::jsonb,
     '2026-05-10T10:00:05Z');

-- The `is_latest` BEFORE-INSERT trigger forces is_latest=TRUE on freshly
-- inserted rows, so we have to demote `s-spawnold` AFTER the insert. The
-- loader's `WHERE is_latest = TRUE` then skips it -> 0 matches for
-- `i-actissold` -> the Issue rewrite falls back to External-legacy with
-- the parent issue id preserved.
UPDATE metis.tasks_v2 SET is_latest = FALSE WHERE id = 's-spawnold';

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
    -- 4. Issue (without matching tasks_v2 row) -> External-legacy("i-actisstwo")
    ('i-actissno', 1, 'task', 'issue actor without match', 'alice',
     '{"Authenticated":{"actor_id":{"Issue":"i-actisstwo"}}}'::jsonb),
    -- 5. Service with valid AgentName -> Agent
    ('i-actsvcok', 1, 'task', 'service-as-agent actor', 'alice',
     '{"Authenticated":{"actor_id":{"Service":"swe"}}}'::jsonb),
    -- 6. Service with invalid AgentName -> External-legacy("has space")
    ('i-actsvcno', 1, 'task', 'service invalid agent name', 'alice',
     '{"Authenticated":{"actor_id":{"Service":"has space"}}}'::jsonb),
    -- 7. Legacy bare string parseable to User
    ('i-actlegu',  1, 'task', 'legacy users/<x>', 'alice',
     '{"Authenticated":{"actor_id":"users/alice"}}'::jsonb),
    -- 8. Legacy bare string parseable to Agent via agents/swe
    ('i-actlega',  1, 'task', 'legacy agents/<x>', 'alice',
     '{"Authenticated":{"actor_id":"agents/swe"}}'::jsonb),
    -- 9. Legacy unparseable bare string -> External-legacy("definitely not an actor")
    ('i-actlegx',  1, 'task', 'legacy unparseable', 'alice',
     '{"Authenticated":{"actor_id":"definitely not an actor"}}'::jsonb),
    -- 10. Already-typed User -> no-op
    ('i-actuser',  1, 'task', 'already-typed User', 'alice',
     '{"Authenticated":{"actor_id":{"User":{"name":"alice"}}}}'::jsonb),
    -- 11. Multi-key map (Legacy catch-all) -> External-legacy(JSON form of the map)
    ('i-actmulti', 1, 'task', 'multi-key actor_id', 'alice',
     '{"Authenticated":{"actor_id":{"kind":"user","name":"alice"}}}'::jsonb),
    -- 12. Legacy adhoc/<sid> -> Adhoc
    ('i-actadhoc', 1, 'task', 'legacy adhoc/<sid>', 'alice',
     '{"Authenticated":{"actor_id":"adhoc/s-adhocone"}}'::jsonb),
    -- 13. Legacy external/<sys>/<user> -> External
    ('i-actextn',  1, 'task', 'legacy external/<sys>/<user>', 'alice',
     '{"Authenticated":{"actor_id":"external/github/jayantk"}}'::jsonb),
    -- 14. Legacy u-<x> shorthand -> User
    ('i-actushrt', 1, 'task', 'legacy u-<x> shorthand', 'alice',
     '{"Authenticated":{"actor_id":"u-alice"}}'::jsonb),
    -- 15. Legacy s-<sid> shorthand -> Adhoc (session_id includes the s- prefix)
    ('i-actsshrt', 1, 'task', 'legacy s-<sid> shorthand', 'alice',
     '{"Authenticated":{"actor_id":"s-abcdef"}}'::jsonb),
    -- 16. Legacy svc-<n> shorthand -> Agent (validates as AgentName)
    ('i-actsvshr', 1, 'task', 'legacy svc-<n> shorthand', 'alice',
     '{"Authenticated":{"actor_id":"svc-swe"}}'::jsonb),
    -- 17. Legacy users/<x> with invalid Username payload -> External-legacy("users/has space")
    ('i-actubad',  1, 'task', 'legacy users/<has space>', 'alice',
     '{"Authenticated":{"actor_id":"users/has space"}}'::jsonb),
    -- 18. Legacy agents/<x> with invalid AgentName payload -> External-legacy("agents/with space")
    ('i-actabad',  1, 'task', 'legacy agents/<with space>', 'alice',
     '{"Authenticated":{"actor_id":"agents/with space"}}'::jsonb),
    -- 19. Legacy external/<sys>/<x> with invalid ExternalSystem -> External-legacy("external/has space/foo")
    ('i-actexbad', 1, 'task', 'legacy external/<has space>/foo', 'alice',
     '{"Authenticated":{"actor_id":"external/has space/foo"}}'::jsonb),
    -- 20. Legacy a-<issue_id> shorthand is intentionally NOT recognised -> External-legacy("a-i-actissone")
    ('i-actashrt', 1, 'task', 'legacy a-<issue_id> shorthand', 'alice',
     '{"Authenticated":{"actor_id":"a-i-actissone"}}'::jsonb),
    -- 21. Multi-match Issue ref -> External-legacy("i-actissmany") (lookup map omits multi-match entries).
    ('i-actrefmny',1, 'task', 'multi-match Issue ref', 'alice',
     '{"Authenticated":{"actor_id":{"Issue":"i-actissmany"}}}'::jsonb),
    -- 22. Deleted-only-task Issue ref -> External-legacy("i-actissdel").
    ('i-actrefdel',1, 'task', 'deleted-task Issue ref', 'alice',
     '{"Authenticated":{"actor_id":{"Issue":"i-actissdel"}}}'::jsonb),
    -- 23. Not-latest-only-task Issue ref -> External-legacy("i-actissold").
    ('i-actrefold',1, 'task', 'not-latest-task Issue ref', 'alice',
     '{"Authenticated":{"actor_id":{"Issue":"i-actissold"}}}'::jsonb),
    -- 24. Chained-Issue Issue ref -> External-legacy("i-actisschn") (lookup chains aren't followed).
    ('i-actrefchn',1, 'task', 'chained-Issue ref', 'alice',
     '{"Authenticated":{"actor_id":{"Issue":"i-actisschn"}}}'::jsonb),
    -- 25. System.on_behalf_of = Username -> resolved to User.
    ('i-actsysu',  1, 'task', 'System.on_behalf_of with Username', 'alice',
     '{"System":{"worker_name":"task-spawner","on_behalf_of":{"Username":"alice"}}}'::jsonb),
    -- 26. System.on_behalf_of = Issue (no match) -> on_behalf_of=null
    -- (whole row stays non-NULL per `actor_variant_cleanup.rs:341-355`).
    ('i-actsysn',  1, 'task', 'System.on_behalf_of with unresolved Issue', 'alice',
     '{"System":{"worker_name":"task-spawner","on_behalf_of":{"Issue":"i-actisstwo"}}}'::jsonb),
    -- 27. Automation.triggered_by = Authenticated/Username -> resolved.
    ('i-actauto',  1, 'task', 'Automation.triggered_by with Username', 'alice',
     '{"Automation":{"automation_name":"github_pr_sync","triggered_by":{"Authenticated":{"actor_id":{"Username":"alice"}}}}}'::jsonb),
    -- 28. Automation.triggered_by = Authenticated/Issue (no match) ->
    -- triggered_by=null (whole row stays non-NULL per
    -- `actor_variant_cleanup.rs:375-384`).
    ('i-actauton', 1, 'task', 'Automation.triggered_by with unresolved Issue', 'alice',
     '{"Automation":{"automation_name":"github_pr_sync","triggered_by":{"Authenticated":{"actor_id":{"Issue":"i-actisstwo"}}}}}'::jsonb);

--------------------------------------------------------------------------------
-- actors_v2.actor + actor_id — the bare ActorId column. The cleanup
-- walks BOTH columns on this table. `actors_v2.actor_id` is NOT NULL
-- since `20260205000000_add_v2_tables.sql`, so the cleanup MUST emit a
-- non-null value for every unmigratable shape — the External-legacy
-- fallback covers that. The last two rows specifically exercise
-- previously-NULLable paths that would have violated the NOT NULL
-- constraint before this fix.
--------------------------------------------------------------------------------
-- `auth_token_hash` / `auth_token_salt` are NOT NULL on `actors_v2`
-- (per `20260205000000_add_v2_tables.sql`) even though both fields are
-- vestigial post-auth-token-table migration. Writes from production
-- code already supply empty strings (see `sqlite_store.rs:756`); the
-- fixture mirrors that pattern.
INSERT INTO metis.actors_v2
    (id, version_number, auth_token_hash, auth_token_salt, creator, actor_id, actor)
VALUES
    ('actu-aname',   1, '', '', 'alice',
     '{"Username":"alice"}'::jsonb,
     '{"Authenticated":{"actor_id":{"Username":"alice"}}}'::jsonb),
    ('actu-asvc',    1, '', '', 'alice',
     '{"Service":"swe"}'::jsonb,
     '{"Authenticated":{"actor_id":{"Service":"swe"}}}'::jsonb),
    -- Issue with no matching tasks_v2 row: previously the cleanup
    -- would NULL `actor_id`, violating NOT NULL. Now it falls back to
    -- External-legacy with the issue id preserved as the username.
    ('actu-aiss',    1, '', '', 'alice',
     '{"Issue":"i-actisstwo"}'::jsonb,
     '{"Authenticated":{"actor_id":{"Issue":"i-actisstwo"}}}'::jsonb),
    -- Service with invalid AgentName: previously NULLed, now falls back
    -- to External-legacy preserving the original `<name>`.
    ('actu-asvcbad', 1, '', '', 'alice',
     '{"Service":"has space"}'::jsonb,
     '{"Authenticated":{"actor_id":{"Service":"has space"}}}'::jsonb);

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

--------------------------------------------------------------------------------
-- Multi-table coverage: the migration walks every table in
-- `ACTOR_REF_TABLES_COMMON` plus session_events_v2 / conversation_events_v2.
-- The pre-existing rows above only exercise issues_v2, actors_v2, and
-- conversation_events_v2; the rows below add one legacy-shape `actor`
-- row to each remaining table so the per-table walker is exercised end
-- to end. Each row picks `{"Username":"alice"}` so the post-cleanup
-- expected shape is the stable `{"User":{"name":"alice"}}` across all
-- tables.
--------------------------------------------------------------------------------

-- repositories_v2.actor
INSERT INTO metis.repositories_v2 (id, version_number, remote_url, actor)
VALUES
    ('r-actreplc', 1, 'https://example.invalid/repo.git',
     '{"Authenticated":{"actor_id":{"Username":"alice"}}}'::jsonb);

-- users_v2.actor
INSERT INTO metis.users_v2 (id, version_number, username, actor)
VALUES
    ('u-actusrlc', 1, 'mtableuser',
     '{"Authenticated":{"actor_id":{"Username":"alice"}}}'::jsonb);

-- patches_v2.actor
INSERT INTO metis.patches_v2
    (id, version_number, description, diff, service_repo_name, creator, actor)
VALUES
    ('p-actpchlc', 1, 'patch with legacy actor', '', 'dourolabs/hydra', 'alice',
     '{"Authenticated":{"actor_id":{"Username":"alice"}}}'::jsonb);

-- tasks_v2.actor — separate from `s-spawnone` (and the other parent-lookup
-- tasks) so spawned_from=NULL keeps it out of the Issue-arm lookup map.
INSERT INTO metis.tasks_v2 (id, version_number, creator, status, deleted, is_latest,
                            spawned_from, conversation_id,
                            mount_spec, agent_config, mode,
                            actor, creation_time)
VALUES
    ('s-actrowx',  1, 'alice', 'complete', FALSE, TRUE,
     NULL, NULL,
     '{"working_dir":"repo","mounts":[]}'::jsonb,
     '{}'::jsonb,
     '{"type":"headless"}'::jsonb,
     '{"Authenticated":{"actor_id":{"Username":"alice"}}}'::jsonb,
     '2026-05-10T10:30:00Z');

-- documents_v2.actor
INSERT INTO metis.documents_v2 (id, version_number, body_markdown, actor)
VALUES
    ('d-actdoclc', 1, '# legacy actor',
     '{"Authenticated":{"actor_id":{"Username":"alice"}}}'::jsonb);

-- session_events_v2.actor — bind to the multi-table tasks_v2 row above.
INSERT INTO metis.session_events_v2
    (session_id, version_number, event_type, event_data, actor, created_at)
VALUES
    ('s-actrowx', 1, 'user_message',
     '{"type":"user_message","content":"se hello","timestamp":"2026-05-10T10:35:00Z"}'::jsonb,
     '{"Authenticated":{"actor_id":{"Username":"alice"}}}'::jsonb,
     '2026-05-10T10:35:00Z');

--------------------------------------------------------------------------------
-- conversations_v2.actor — the cleanup walks this table too. The prod
-- failure mode for [[i-jyhvstcj]] was a `{"Session":"s-..."}`-tagged
-- actor surviving into `GET /v1/conversations`; the row below seeds
-- that exact shape so the §3.3 store-level smoke proves the rewrite
-- unblocks `get_conversation`.
--------------------------------------------------------------------------------
INSERT INTO metis.conversations_v2 (id, version_number, creator, actor)
VALUES
    ('c-actconvx', 1, 'alice',
     '{"Authenticated":{"actor_id":{"Session":"s-csessacx"}}}'::jsonb);

--------------------------------------------------------------------------------
-- issues_v2.form_response — JSONB blob with an embedded `actor: ActorId`
-- field. The cleanup walks `.actor` while preserving sibling fields
-- (`action_id`, `values`, `submitted_at`). This is the second prod
-- failure mode for [[i-jyhvstcj]] — `Username` survived into
-- `GET /v1/issues` form_response deserialization.
--------------------------------------------------------------------------------
INSERT INTO metis.issues_v2 (id, version_number, issue_type, description, creator, form_response)
VALUES
    ('i-actform',  1, 'task', 'form_response with Username actor', 'alice',
     '{"action_id":"approve","actor":{"Username":"alice"},"values":{"score":4},"submitted_at":"2026-05-10T11:00:00Z"}'::jsonb);
