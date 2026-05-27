-- baseline-version: 20260519000000
-- Pre-actor-overhaul baseline shapes: bare-string / `users/`-prefixed /
-- `agents/`-prefixed assignees on issues_v2, bare and typed review authors on
-- patches_v2, conversation_events_v2 rows the events Rust migration moves into
-- session_events_v2, snake_case `refers_to` relationships, legacy auth_tokens.
-- INSERTs are valid against the schema state at version `20260519000000`
-- (immediately after `20260519000000_add_task_usage.sql` applies, and before
-- the actor-overhaul / sessions-orthogonality / merge-policy / created-by-drops
-- / refers-to-rename batch of migrations this release validates).
--
-- Each row exercises a source shape that one of the migrations under test
-- rewrites or relies on; the corresponding destination assertion lives in
-- `hydra-server/tests/migration_roundtrip.rs`.

--------------------------------------------------------------------------------
-- issues_v2 — assignee source shapes for the assignee_principal backfill
--------------------------------------------------------------------------------
INSERT INTO metis.issues_v2 (id, version_number, issue_type, description, creator, assignee)
VALUES
    ('i-bare000001',     1, 'task', 'bare-string assignee',     'jayantk', 'jayantk'),
    ('i-userpfx0001',    1, 'task', 'users/-prefixed assignee', 'jayantk', 'users/jayantk'),
    ('i-agentpfx001',    1, 'task', 'agents/-prefixed assignee','jayantk', 'agents/swe'),
    ('i-extslash001',    1, 'task', 'external/<sys>/<x> assignee (left NULL by migration)', 'jayantk', 'external/github/foo'),
    ('i-nullasgn01',     1, 'task', 'null assignee',            'jayantk', NULL);

--------------------------------------------------------------------------------
-- patches_v2 — review author source shapes for review_author_principal rewrite
--   * one review with bare-string author     ('jayantk')
--   * one review with agents/-prefixed author ('agents/swe')
--   * one review with an already-typed Principal object (must remain unchanged)
--------------------------------------------------------------------------------
INSERT INTO metis.patches_v2 (id, version_number, description, diff, service_repo_name, reviews)
VALUES
    ('p-bareauth01', 1, 'review author is a bare username', '', 'dourolabs/hydra',
     '[{"author":"jayantk","contents":"lgtm","is_approved":true,"submitted_at":"2026-05-01T00:00:00Z"}]'::jsonb),
    ('p-agentauth1', 1, 'review author is agents/-prefixed', '', 'dourolabs/hydra',
     '[{"author":"agents/swe","contents":"approve","is_approved":true,"submitted_at":"2026-05-02T00:00:00Z"}]'::jsonb),
    ('p-typedauth1', 1, 'review author is already a typed Principal', '', 'dourolabs/hydra',
     '[{"author":{"User":{"name":"jayantk"}},"contents":"already typed","is_approved":true,"submitted_at":"2026-05-03T00:00:00Z"}]'::jsonb);

--------------------------------------------------------------------------------
-- tasks_v2 — exercise the session-shape backfill (each mode).
--   * headless task: no conversation_id  -> mode {"type":"headless", ...}
--   * interactive task: conversation_id set + interactive=true -> mode {"type":"interactive", ...}
--   * resumed task: conversation_resume_from set -> resumed_from backfill targets the predecessor
--
-- All three need `prompt` and `context` (both NOT NULL at the baseline pin;
-- dropped by 20260525000000_drop_legacy_session_columns).
--
-- creation_time is set so the resumed_from backfill subquery (which orders by
-- creation_time) has a deterministic predecessor.
--------------------------------------------------------------------------------
INSERT INTO metis.tasks_v2 (id, version_number, prompt, context, interactive, conversation_id, conversation_resume_from, creation_time)
VALUES
    ('s-headless01', 1, 'do a thing',
     '{"type":"none"}'::jsonb,
     false, NULL, NULL, '2026-05-10T10:00:00Z'),
    ('s-interact01', 1, 'chat',
     '{"type":"git_repository","remote_url":"https://example.invalid/repo.git"}'::jsonb,
     true,  'c-conv00001', NULL, '2026-05-10T11:00:00Z'),
    ('s-interact02', 1, 'chat continued',
     '{"type":"git_repository","remote_url":"https://example.invalid/repo.git"}'::jsonb,
     true,  'c-conv00001', 1,    '2026-05-10T12:00:00Z');

--------------------------------------------------------------------------------
-- conversations_v2 — the parent conversation for the tasks/events above
--------------------------------------------------------------------------------
INSERT INTO metis.conversations_v2 (id, version_number, creator)
VALUES ('c-conv00001', 1, 'jayantk');

--------------------------------------------------------------------------------
-- conversation_events_v2 — populated event rows whose `event_data` matches the
-- `SessionEvent` enum shape on disk. The `migrate-events` external migration
-- (hooked in `run_external_migrations`) copies these into `session_events_v2`
-- verbatim, and `Store::get_session_events` then deserializes them back into
-- typed `SessionEvent` variants for the §3.3 smoke. Explicit `created_at`
-- timestamps anchor the rows inside s-interact01's window ([11:00, 12:00) at
-- the baseline tasks_v2 creation_time values above) so the partitioning is
-- deterministic across hosts.
--------------------------------------------------------------------------------
INSERT INTO metis.conversation_events_v2 (conversation_id, version_number, event_type, event_data, created_at)
VALUES
    ('c-conv00001', 1, 'user_message',
     '{"type":"user_message","content":"hello","timestamp":"2026-05-10T11:15:00Z"}'::jsonb,
     '2026-05-10T11:15:00Z'),
    ('c-conv00001', 2, 'assistant_message',
     '{"type":"assistant_message","content":"hi","timestamp":"2026-05-10T11:30:00Z"}'::jsonb,
     '2026-05-10T11:30:00Z');

--------------------------------------------------------------------------------
-- object_relationships — snake_case `refers_to` becomes kebab-case `refers-to`
-- after 20260529000000_rename_refers_to_to_kebab_case. Include an already-
-- kebab `has-patch` row to confirm the rename only touches the snake variant.
--------------------------------------------------------------------------------
INSERT INTO metis.object_relationships (source_id, source_kind, target_id, target_kind, rel_type)
VALUES
    ('i-bare000001', 'issue', 'i-userpfx0001', 'issue', 'refers_to'),
    ('i-bare000001', 'issue', 'p-bareauth01',  'patch', 'has-patch');

--------------------------------------------------------------------------------
-- auth_tokens — legacy row predating session_id / is_revoked columns; the
-- migrations that add those columns must populate them (session_id stays NULL,
-- is_revoked defaults to FALSE).
--------------------------------------------------------------------------------
INSERT INTO metis.auth_tokens (actor_name, token_hash)
VALUES ('agents/swe', 'deadbeef');

--------------------------------------------------------------------------------
-- repositories_v2 — row carries a populated `patch_workflow` (still a column
-- at baseline); the column is dropped by 20260523030000 and the §3.1
-- invariant verifies the drop. `merge_policy` is added by 20260523000000.
--------------------------------------------------------------------------------
INSERT INTO metis.repositories_v2 (id, version_number, remote_url, patch_workflow)
VALUES ('r-repo00001', 1, 'https://example.invalid/repo.git',
        '{"reviewers":["jayantk"]}'::jsonb);

--------------------------------------------------------------------------------
-- documents_v2 — `created_by` was a real column at baseline and is dropped by
-- 20260527000001_drop_documents_created_by. A populated value here confirms
-- the column-drop migration tolerates non-NULL existing rows.
--------------------------------------------------------------------------------
INSERT INTO metis.documents_v2 (id, version_number, body_markdown, created_by)
VALUES ('d-doc000001', 1, '# hello', 's-headless01');

--------------------------------------------------------------------------------
-- §3.3 store-level smoke rows. The pre-existing rows above use IDs with digits
-- (e.g. `i-bare000001`); those are fine for the §3.1 / §3.2 SQL-level
-- assertions which read them back through raw `sqlx::query`, but the Rust
-- `IssueId` / `PatchId` / `SessionId` newtypes reject any non-alphabetic
-- suffix, so calling `Store::get_issue(&"i-bare000001".parse()?)` errors at
-- parse time before the test can exercise the typed deserialization path. We
-- add a parallel set of rows below with all-alphabetic suffixes so the PR-2
-- §3.3 smoke can walk each (assignee shape × Principal variant), each
-- (review-author shape × Principal variant), each `SessionMode` variant, the
-- renamed `refers-to` relationship, and the migrated `session_events_v2` rows
-- through the live Store APIs and confirm the typed domain objects round-trip
-- as expected.
--------------------------------------------------------------------------------

-- issues_v2 — one row per source-shape assignee that the principal backfill
-- handles.
INSERT INTO metis.issues_v2 (id, version_number, issue_type, description, creator, assignee)
VALUES
    ('i-bareasgn',   1, 'task', 'bare-string assignee',     'jayantk', 'jayantk'),
    ('i-userpath',   1, 'task', 'users/-prefixed assignee', 'jayantk', 'users/jayantk'),
    ('i-agentpath',  1, 'task', 'agents/-prefixed assignee','jayantk', 'agents/swe'),
    ('i-extpath',    1, 'task', 'external/<sys>/<x> assignee (left NULL by migration)', 'jayantk', 'external/github/foo'),
    ('i-nullasgn',   1, 'task', 'null assignee',            'jayantk', NULL);

-- patches_v2 — one row per review-author source shape.
INSERT INTO metis.patches_v2 (id, version_number, description, diff, service_repo_name, reviews)
VALUES
    ('p-barerev',    1, 'review author is a bare username', '', 'dourolabs/hydra',
     '[{"author":"jayantk","contents":"lgtm","is_approved":true,"submitted_at":"2026-05-01T00:00:00Z"}]'::jsonb),
    ('p-agentrev',   1, 'review author is agents/-prefixed', '', 'dourolabs/hydra',
     '[{"author":"agents/swe","contents":"approve","is_approved":true,"submitted_at":"2026-05-02T00:00:00Z"}]'::jsonb),
    ('p-typedrev',   1, 'review author is already a typed Principal', '', 'dourolabs/hydra',
     '[{"author":{"User":{"name":"jayantk"}},"contents":"already typed","is_approved":true,"submitted_at":"2026-05-03T00:00:00Z"}]'::jsonb);

-- conversations_v2 — parent conversation for the §3.3 sessions / events.
INSERT INTO metis.conversations_v2 (id, version_number, creator)
VALUES ('c-convalpha', 1, 'jayantk');

-- tasks_v2 — one row per `SessionMode` shape (Headless, Interactive, and a
-- resumed interactive that the resumed_from backfill chains onto its
-- predecessor in the same conversation).
INSERT INTO metis.tasks_v2 (id, version_number, prompt, context, interactive, conversation_id, conversation_resume_from, creation_time)
VALUES
    ('s-headalpha', 1, 'do a thing',
     '{"type":"none"}'::jsonb,
     false, NULL, NULL, '2026-05-10T13:00:00Z'),
    ('s-interone',  1, 'chat',
     '{"type":"git_repository","remote_url":"https://example.invalid/repo.git"}'::jsonb,
     true,  'c-convalpha', NULL, '2026-05-10T14:00:00Z'),
    ('s-intertwo',  1, 'chat continued',
     '{"type":"git_repository","remote_url":"https://example.invalid/repo.git"}'::jsonb,
     true,  'c-convalpha', 1,    '2026-05-10T15:00:00Z');

-- conversation_events_v2 — message rows for c-convalpha with `SessionEvent`-
-- shaped event_data so the migrate-events smoke can read them back through
-- `Store::get_session_events` and deserialize them into typed
-- `SessionEvent::UserMessage` / `AssistantMessage` variants. Timestamps land
-- inside s-interone's window ([14:00, 15:00) per the s-intertwo creation
-- time) so they partition deterministically onto s-interone.
INSERT INTO metis.conversation_events_v2 (conversation_id, version_number, event_type, event_data, created_at)
VALUES
    ('c-convalpha', 1, 'user_message',
     '{"type":"user_message","content":"smoke hello","timestamp":"2026-05-10T14:15:00Z"}'::jsonb,
     '2026-05-10T14:15:00Z'),
    ('c-convalpha', 2, 'assistant_message',
     '{"type":"assistant_message","content":"smoke hi","timestamp":"2026-05-10T14:30:00Z"}'::jsonb,
     '2026-05-10T14:30:00Z');

-- object_relationships — a snake_case `refers_to` row between the new issues
-- so the rename migration converts it to `refers-to`, then the §3.3 smoke
-- reads it back through `Store::get_relationships(..., Some(RefersTo))` with
-- parseable HydraId endpoints.
INSERT INTO metis.object_relationships (source_id, source_kind, target_id, target_kind, rel_type)
VALUES
    ('i-bareasgn', 'issue', 'i-userpath', 'issue', 'refers_to');

--------------------------------------------------------------------------------
-- migrate-events partitioning edge cases. These exercise the tolerant
-- assignment rules added in `events.rs`:
--
--   * `c-convnoses`  — conversation has message events but no linked
--                      sessions in tasks_v2. Migration must NOT bail; the
--                      message rows are dropped with a WARN log.
--   * `c-convbefore` — single session with an event whose `created_at`
--                      predates the session's `creation_time`. The event
--                      lands on the first (and only) session.
--   * `c-convgap`    — two sessions with a gap between A's suspend and B's
--                      creation; an event in that gap lands on B (the
--                      subsequent session), NOT A.
--   * `c-convafter`  — two sessions where the LAST one is suspended; an
--                      event past the suspend timestamp lands on the last
--                      session.
--
-- Matching assertions live in `assert_events_migration_edge_cases` in
-- `tests/migration_roundtrip.rs`. SessionIds use all-alphabetic suffixes
-- so they parse through the Rust `SessionId` newtype if any future smoke
-- assertion wants to read them back via the Store API.
--------------------------------------------------------------------------------

-- 1. No-sessions conversation. Parent row in conversations_v2 keeps the FK
--    alive; the message event is what the migration must tolerate.
INSERT INTO metis.conversations_v2 (id, version_number, creator)
VALUES ('c-convnoses', 1, 'jayantk');
INSERT INTO metis.conversation_events_v2 (conversation_id, version_number, event_type, event_data, created_at)
VALUES
    ('c-convnoses', 1, 'user_message',
     '{"type":"user_message","content":"orphan hello","timestamp":"2026-05-10T15:00:00Z"}'::jsonb,
     '2026-05-10T15:00:00Z');

-- 2. Events-before-all-sessions: the single session s-beforeone starts at
--    16:00; the message at 15:30 falls before its window and must still
--    land on s-beforeone.
INSERT INTO metis.conversations_v2 (id, version_number, creator)
VALUES ('c-convbefore', 1, 'jayantk');
INSERT INTO metis.tasks_v2 (id, version_number, prompt, context, interactive, conversation_id, conversation_resume_from, creation_time)
VALUES
    ('s-beforeone', 1, 'chat',
     '{"type":"git_repository","remote_url":"https://example.invalid/repo.git"}'::jsonb,
     true,  'c-convbefore', NULL, '2026-05-10T16:00:00Z');
INSERT INTO metis.conversation_events_v2 (conversation_id, version_number, event_type, event_data, created_at)
VALUES
    ('c-convbefore', 1, 'user_message',
     '{"type":"user_message","content":"early bird","timestamp":"2026-05-10T15:30:00Z"}'::jsonb,
     '2026-05-10T15:30:00Z');

-- 3. Gap-between-sessions: s-gapone created at 17:00 then suspended at
--    17:30; s-gaptwo created at 18:00; the event at 17:45 sits in the
--    [17:30, 18:00) gap and must land on s-gaptwo (the subsequent session).
--    The `actor` JSON shape mirrors what
--    `serde_json::to_string(&ActorRef::Authenticated { actor_id:
--     ActorId::Session(SessionId("s-gapone")), session_id: None })`
--    produces (externally-tagged Principal/ActorRef wire shape).
INSERT INTO metis.conversations_v2 (id, version_number, creator)
VALUES ('c-convgap', 1, 'jayantk');
INSERT INTO metis.tasks_v2 (id, version_number, prompt, context, interactive, conversation_id, conversation_resume_from, creation_time)
VALUES
    ('s-gapone', 1, 'chat',
     '{"type":"git_repository","remote_url":"https://example.invalid/repo.git"}'::jsonb,
     true,  'c-convgap', NULL, '2026-05-10T17:00:00Z'),
    ('s-gaptwo', 1, 'chat resumed',
     '{"type":"git_repository","remote_url":"https://example.invalid/repo.git"}'::jsonb,
     true,  'c-convgap', 1,    '2026-05-10T18:00:00Z');
INSERT INTO metis.conversation_events_v2 (conversation_id, version_number, event_type, event_data, actor, created_at)
VALUES
    ('c-convgap', 1, 'suspending',
     '{"type":"suspending","reason":"test","timestamp":"2026-05-10T17:30:00Z"}'::jsonb,
     '{"Authenticated":{"actor_id":{"Session":"s-gapone"}}}'::jsonb,
     '2026-05-10T17:30:00Z'),
    ('c-convgap', 2, 'user_message',
     '{"type":"user_message","content":"in the gap","timestamp":"2026-05-10T17:45:00Z"}'::jsonb,
     NULL,
     '2026-05-10T17:45:00Z');

-- 4. Event-after-last-session: s-afterone created at 19:00, s-aftertwo at
--    20:00; s-aftertwo suspends at 20:30. The event at 20:45 sits past the
--    last session's suspend and must land on s-aftertwo (the last session).
INSERT INTO metis.conversations_v2 (id, version_number, creator)
VALUES ('c-convafter', 1, 'jayantk');
INSERT INTO metis.tasks_v2 (id, version_number, prompt, context, interactive, conversation_id, conversation_resume_from, creation_time)
VALUES
    ('s-afterone', 1, 'chat',
     '{"type":"git_repository","remote_url":"https://example.invalid/repo.git"}'::jsonb,
     true,  'c-convafter', NULL, '2026-05-10T19:00:00Z'),
    ('s-aftertwo', 1, 'chat resumed',
     '{"type":"git_repository","remote_url":"https://example.invalid/repo.git"}'::jsonb,
     true,  'c-convafter', 1,    '2026-05-10T20:00:00Z');
INSERT INTO metis.conversation_events_v2 (conversation_id, version_number, event_type, event_data, actor, created_at)
VALUES
    ('c-convafter', 1, 'suspending',
     '{"type":"suspending","reason":"test","timestamp":"2026-05-10T20:30:00Z"}'::jsonb,
     '{"Authenticated":{"actor_id":{"Session":"s-aftertwo"}}}'::jsonb,
     '2026-05-10T20:30:00Z'),
    ('c-convafter', 2, 'user_message',
     '{"type":"user_message","content":"past the end","timestamp":"2026-05-10T20:45:00Z"}'::jsonb,
     NULL,
     '2026-05-10T20:45:00Z');
