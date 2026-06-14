-- baseline-version: 20260716000000
-- SQLite pre-seed-progress-as-comments baseline. INSERTs are valid
-- against the schema state after sqlite migration
-- `20260716000000_add_statuses_archived.sql`, immediately before
-- `20260720000000_seed_progress_as_comments_anchor.sql` (the no-op
-- SQL anchor for the `seed-progress-as-comments` Rust migration).
-- Sister to the postgres baseline at the same version.
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
--
-- Idempotency: the `seed-progress-as-comments` migration re-runs on
-- every server boot; this fixture is the substrate for the
-- skip-if-already-seeded check.

-- Issues land on the seeded default project (`j-defaul`, status
-- key `open`, sequence 1). `status_sequence = 1` resolves through
-- the `(project_id, sequence)` FK to the seeded `open` row.

INSERT INTO issues_v2 (
    id, version_number, issue_type, title, description, creator,
    progress, status_sequence, deleted, actor, created_at, updated_at,
    feedback, is_latest, project_id
) VALUES
    ('i-progseed1', 1, 'task', 'progress seed (two versions, v1)',
     'fixture issue: first version', 'alice',
     'first progress note', 1, 0,
     '{"Authenticated":{"actor_id":{"User":{"name":"alice"}}}}',
     '2026-07-01T10:00:00.000+00:00',
     '2026-07-01T10:00:00.000+00:00',
     NULL, 0, 'j-defaul'),
    ('i-progseed1', 2, 'task', 'progress seed (two versions, v2)',
     'fixture issue: second version', 'alice',
     'latest progress note', 1, 0,
     '{"Authenticated":{"actor_id":{"User":{"name":"bob"}}}}',
     '2026-07-02T11:30:00.000+00:00',
     '2026-07-02T11:30:00.000+00:00',
     NULL, 1, 'j-defaul'),
    ('i-progseed2', 1, 'task', 'progress seed (single version)',
     'fixture issue: single version', 'alice',
     'only progress note', 1, 0,
     '{"Authenticated":{"actor_id":{"User":{"name":"carol"}}}}',
     '2026-07-03T09:00:00.000+00:00',
     '2026-07-03T09:00:00.000+00:00',
     NULL, 1, 'j-defaul'),
    ('i-progfb', 1, 'task', 'feedback only (no progress)',
     'fixture issue: feedback set, progress empty', 'alice',
     '', 1, 0,
     '{"Authenticated":{"actor_id":{"User":{"name":"alice"}}}}',
     '2026-07-04T08:00:00.000+00:00',
     '2026-07-04T08:00:00.000+00:00',
     'please redirect this work', 1, 'j-defaul');
