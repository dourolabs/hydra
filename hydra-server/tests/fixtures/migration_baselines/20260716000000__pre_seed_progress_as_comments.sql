-- baseline-version: 20260716000000
-- Postgres pre-seed-progress-as-comments baseline. Sister to the
-- sqlite baseline at the same version. INSERTs are valid against the
-- schema state at sqlx migration
-- `20260716000000_add_statuses_archived.sql` and immediately before
-- `20260720000000_seed_progress_as_comments_anchor.sql` (the no-op
-- SQL anchor for the `seed-progress-as-comments` Rust migration).
--
-- Seeds three issues exercising the migration's branches:
--   * `i-progseed1`: two versions; v1 sets progress=A, v2 changes it
--     to B. The seeded comment must have body B, attributed to v2's
--     actor and timestamp.
--   * `i-progseed2`: one version with non-empty progress. The seeded
--     comment must have that body, with the v1 actor/timestamp.
--   * `i-progfb`: populates `feedback` but leaves `progress` empty.
--     Per spec, feedback is dropped outright — no comment must be
--     seeded for this issue.

INSERT INTO metis.issues_v2 (
    id, version_number, issue_type, title, description, creator,
    progress, status_sequence, deleted, actor, created_at, updated_at,
    feedback, is_latest, project_id
) VALUES
    ('i-progseed1', 1, 'task', 'progress seed (two versions, v1)',
     'fixture issue: first version', 'alice',
     'first progress note', 1, FALSE,
     '{"Authenticated":{"actor_id":{"User":{"name":"alice"}}}}'::jsonb,
     '2026-07-01T10:00:00+00:00',
     '2026-07-01T10:00:00+00:00',
     NULL, FALSE, 'j-defaul'),
    ('i-progseed1', 2, 'task', 'progress seed (two versions, v2)',
     'fixture issue: second version', 'alice',
     'latest progress note', 1, FALSE,
     '{"Authenticated":{"actor_id":{"User":{"name":"bob"}}}}'::jsonb,
     '2026-07-02T11:30:00+00:00',
     '2026-07-02T11:30:00+00:00',
     NULL, TRUE, 'j-defaul'),
    ('i-progseed2', 1, 'task', 'progress seed (single version)',
     'fixture issue: single version', 'alice',
     'only progress note', 1, FALSE,
     '{"Authenticated":{"actor_id":{"User":{"name":"carol"}}}}'::jsonb,
     '2026-07-03T09:00:00+00:00',
     '2026-07-03T09:00:00+00:00',
     NULL, TRUE, 'j-defaul'),
    ('i-progfb', 1, 'task', 'feedback only (no progress)',
     'fixture issue: feedback set, progress empty', 'alice',
     '', 1, FALSE,
     '{"Authenticated":{"actor_id":{"User":{"name":"alice"}}}}'::jsonb,
     '2026-07-04T08:00:00+00:00',
     '2026-07-04T08:00:00+00:00',
     'please redirect this work', TRUE, 'j-defaul');
