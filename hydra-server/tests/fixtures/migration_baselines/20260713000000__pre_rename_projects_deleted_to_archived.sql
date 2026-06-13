-- baseline-version: 20260713000000
-- Postgres pre-rename-projects-deleted-to-archived baseline. Sister to
-- the sqlite baseline at the same version. INSERTs are valid against
-- the schema state at sqlx migration
-- `20260713000000_add_statuses_session_settings.sql` and immediately
-- before `20260715000000_rename_projects_deleted_to_archived.sql`.
--
-- Seeds two projects at the OLD `metis.projects.deleted` column shape
-- so the post-rename roundtrip assertions can verify that the rename
-- preserves the row's flag verbatim and the value surfaces as
-- `Project.archived` through the current Store API.

-- `next_status_sequence` must strictly exceed MAX(statuses.sequence)
-- for the project (the cutover invariant), so set it to 2 since each
-- baseline project below carries a single status at sequence 1.

-- Soft-deleted row: post-rename, `archived` must be true and the
-- round-trip through `PostgresStoreV2::get_project(.., true)` must
-- surface `Project.archived == true`.
INSERT INTO metis.projects (
    id, version_number, key, name, creator,
    deleted, prompt_path, next_status_sequence
)
VALUES (
    'j-renarcha', 1, 'rename-archived', 'Rename Archived', 'jayantk',
    TRUE, NULL, 2
);

-- Live row: post-rename, `archived` must be false; the row keeps
-- surfacing through `list_projects(false)`.
INSERT INTO metis.projects (
    id, version_number, key, name, creator,
    deleted, prompt_path, next_status_sequence
)
VALUES (
    'j-renarchb', 1, 'rename-not-archived', 'Rename Not Archived', 'jayantk',
    FALSE, NULL, 2
);

-- Explicit `position = sequence` so the
-- `add_statuses_position_backfills_to_sequence` invariant holds for
-- these post-backfill INSERTs. The other status columns added by
-- newer migrations (auto_archive_after_seconds,
-- max_simultaneous_sessions, suppress_sessions, session_settings_json)
-- can fall through to their column defaults.
INSERT INTO metis.statuses (
    project_id, sequence, key, label, color,
    unblocks_parents, unblocks_dependents, cascades_to_children,
    on_enter, prompt_path, interactive, position
) VALUES
    ('j-renarcha', 1, 'todo', 'Todo', '#abcdef', FALSE, FALSE, FALSE, NULL, NULL, FALSE, 1.0),
    ('j-renarchb', 1, 'todo', 'Todo', '#abcdef', FALSE, FALSE, FALSE, NULL, NULL, FALSE, 1.0);
