-- baseline-version: 20260519000000
-- SQLite pre-actor-overhaul baseline. INSERTs are valid against the schema state
-- at sqlite migration `20260519000000_add_task_usage.sql`, before this release's
-- assignee_principal / review.author / refers-to-rename / agent_config-system-
-- prompt backfills run. Sister to `migration_baselines/20260519000000__pre_actor_overhaul.sql`
-- (the postgres baseline) — kept independent so postgres-only fixture changes
-- don't ripple here.
--
-- Scope: four backfill migrations that are byte-for-byte equivalent across both
-- backends but were previously only asserted on the postgres side
-- ([[i-uazczsbc]]):
--
--   * 20260530000000_add_assignee_principal_to_issues.sql
--   * 20260601000000_review_author_principal.sql
--   * 20260529000000_rename_refers_to_to_kebab_case.sql
--   * 20260603010000_backfill_agent_config_system_prompt.sql
--
-- SQLite differences vs. the postgres baseline:
--   * No `metis.` schema prefix.
--   * Boolean columns are INTEGERs (0/1); no triggers backfill `is_latest`,
--     so we set it explicitly on every row.
--   * JSON columns are TEXT (no `::jsonb`).
--   * `tasks_v2.creator` is nullable at this pin but tightened to NOT NULL
--     by 20260602000000_require_creator_not_null; we populate it on every
--     row so the post-tighten state is satisfied.

-- Parent conversation referenced by the interactive sessions below.
INSERT INTO conversations (id, version_number, creator, is_latest)
VALUES ('c-convalpha', 1, 'jayantk', 1);

--------------------------------------------------------------------------------
-- issues_v2 — one row per assignee source shape the 20260530000000
-- assignee_principal backfill handles. external/<sys>/<x> and NULL assignees
-- are intentionally left with NULL assignee_principal by the migration.
--------------------------------------------------------------------------------
INSERT INTO issues_v2 (id, version_number, issue_type, description, creator, assignee, is_latest)
VALUES
    ('i-bareasgn',  1, 'task', 'bare-string assignee',     'jayantk', 'jayantk',             1),
    ('i-userpath',  1, 'task', 'users/-prefixed assignee', 'jayantk', 'users/jayantk',       1),
    ('i-agentpath', 1, 'task', 'agents/-prefixed assignee','jayantk', 'agents/swe',          1),
    ('i-extpath',   1, 'task', 'external/<sys>/<x> assignee (left NULL by backfill)', 'jayantk', 'external/github/foo', 1),
    ('i-nullasgn',  1, 'task', 'null assignee',            'jayantk', NULL,                  1);

--------------------------------------------------------------------------------
-- patches_v2 — one row per review-author source shape the 20260601000000
-- review_author_principal rewrite handles.
--------------------------------------------------------------------------------
INSERT INTO patches_v2 (id, version_number, description, diff, service_repo_name, reviews, is_latest)
VALUES
    ('p-barerev',  1, 'review author is a bare username', '', 'dourolabs/hydra',
     '[{"author":"jayantk","contents":"lgtm","is_approved":true,"submitted_at":"2026-05-01T00:00:00Z"}]', 1),
    ('p-agentrev', 1, 'review author is agents/-prefixed', '', 'dourolabs/hydra',
     '[{"author":"agents/swe","contents":"approve","is_approved":true,"submitted_at":"2026-05-02T00:00:00Z"}]', 1),
    ('p-typedrev', 1, 'review author is already a typed Principal', '', 'dourolabs/hydra',
     '[{"author":{"User":{"name":"jayantk"}},"contents":"already typed","is_approved":true,"submitted_at":"2026-05-03T00:00:00Z"}]', 1);

--------------------------------------------------------------------------------
-- tasks_v2 — exercise the session-shape backfill (20260523020000) and the
-- system_prompt backfill follow-up (20260603010000). `prompt` and `context`
-- are NOT NULL at this pin; both are dropped by 20260525000000. The headless
-- row's `prompt` rides through `mode.prompt` (added by 20260523020000) and
-- onto `agent_config.system_prompt` (set by 20260603010000).
--------------------------------------------------------------------------------
INSERT INTO tasks_v2 (id, version_number, prompt, context, interactive, conversation_id, conversation_resume_from, creator, status, deleted, is_latest, creation_time)
VALUES
    ('s-headalpha', 1, 'do a thing',
     '{"type":"none"}',
     0, NULL, NULL, 'jayantk', 'complete', 0, 1, '2026-05-10T13:00:00Z'),
    ('s-interone',  1, 'chat',
     '{"type":"git_repository","remote_url":"https://example.invalid/repo.git"}',
     1, 'c-convalpha', NULL, 'jayantk', 'complete', 0, 1, '2026-05-10T14:00:00Z'),
    ('s-intertwo',  1, 'chat continued',
     '{"type":"git_repository","remote_url":"https://example.invalid/repo.git"}',
     1, 'c-convalpha', 1, 'jayantk', 'complete', 0, 1, '2026-05-10T15:00:00Z');

--------------------------------------------------------------------------------
-- object_relationships — snake_case `refers_to` row that the
-- 20260529000000_rename_refers_to_to_kebab_case migration must convert to
-- `refers-to`. The §3.3 smoke reads it back through
-- `SqliteStore::get_relationships` with `RelationshipType::RefersTo`.
--------------------------------------------------------------------------------
INSERT INTO object_relationships (source_id, source_kind, target_id, target_kind, rel_type)
VALUES
    ('i-bareasgn', 'issue', 'i-userpath', 'issue', 'refers_to');
