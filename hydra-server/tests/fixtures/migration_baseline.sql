-- baseline-version: 20260519000000
-- Hand-curated baseline fixture for the migration roundtrip test. The header
-- comment on line 1 must be `-- baseline-version: <N>`; the test parses it via
-- `parse_baseline_pin` to decide which sqlx migrations to apply before loading
-- the body below. The remainder is `INSERT` statements valid against the
-- `hydra-server` schema at that pin (i.e. just after
-- `20260519000000_add_task_usage.sql` and before the actor-overhaul /
-- sessions-orthogonality / merge-policy / created-by-drops / refers-to-rename
-- batch of migrations this release validates).
--
-- Each row exercises a source shape that one of the migrations under test
-- rewrites or relies on; the corresponding destination assertion lives in
-- `hydra-server/tests/migration_roundtrip.rs`.
--
-- PR-3's regen tool will replace this hand-curated file with a `pg_dump`
-- equivalent and add a `-- migrations-hash: <hex>` line on the next release.

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
     '[{"author":{"kind":"user","name":"jayantk"},"contents":"already typed","is_approved":true,"submitted_at":"2026-05-03T00:00:00Z"}]'::jsonb);

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
-- conversation_events_v2 — populated event rows. PR-2 will exercise the
-- migrate-events external migration; PR-1 just needs them present so the
-- §3.1 invariant for `session_events_v2` existence has something to count.
--------------------------------------------------------------------------------
INSERT INTO metis.conversation_events_v2 (conversation_id, version_number, event_type, event_data)
VALUES
    ('c-conv00001', 1, 'user_message',      '{"text":"hello"}'::jsonb),
    ('c-conv00001', 2, 'assistant_message', '{"text":"hi"}'::jsonb);

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
