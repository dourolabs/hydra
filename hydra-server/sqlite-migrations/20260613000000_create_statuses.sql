-- Sister to the Postgres `20260613000000_create_statuses.sql`. Creates
-- the `statuses` table and backfills it from `projects.statuses` JSON
-- on every latest-version project row. See [[i-bqimglba]] for design.
--
-- SQLite differences vs. Postgres:
--   * No `metis.` schema prefix.
--   * BOOLEAN columns are INTEGER (0/1).
--   * JSONB columns are TEXT — `on_enter` stores the JSON-encoded
--     `StatusOnEnter` blob as text (NULL when None).
--   * Sequence id is INTEGER instead of BIGINT (SQLite collapses both
--     to a 64-bit signed int).
--   * `json_each` exposes the array entries with a 0-based `key`; we
--     add 1 to match the Postgres `WITH ORDINALITY` 1-indexed `ord`.

CREATE TABLE IF NOT EXISTS statuses (
    project_id           TEXT    NOT NULL,
    sequence             INTEGER NOT NULL,
    key                  TEXT    NOT NULL,
    label                TEXT    NOT NULL,
    color                TEXT    NOT NULL,
    unblocks_parents     INTEGER NOT NULL,
    unblocks_dependents  INTEGER NOT NULL,
    cascades_to_children INTEGER NOT NULL,
    on_enter             TEXT    NULL,
    prompt_path          TEXT    NULL,
    interactive          INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (project_id, sequence)
);

CREATE UNIQUE INDEX IF NOT EXISTS statuses_project_key_idx
    ON statuses (project_id, key);

-- Backfill from each latest-version project's JSON statuses array.
-- `json_extract(elem.value, '$.on_enter')` returns SQL NULL for both
-- missing keys and explicit JSON null, so no additional NULLIF dance
-- is needed for `on_enter`. `INSERT OR IGNORE` makes the statement
-- idempotent if the body is ever re-applied outside the migration
-- tracker.
INSERT OR IGNORE INTO statuses (
    project_id,
    sequence,
    key,
    label,
    color,
    unblocks_parents,
    unblocks_dependents,
    cascades_to_children,
    on_enter,
    prompt_path,
    interactive
)
SELECT
    p.id,
    elem.key + 1,
    json_extract(elem.value, '$.key'),
    json_extract(elem.value, '$.label'),
    json_extract(elem.value, '$.color'),
    json_extract(elem.value, '$.unblocks_parents'),
    json_extract(elem.value, '$.unblocks_dependents'),
    json_extract(elem.value, '$.cascades_to_children'),
    json_extract(elem.value, '$.on_enter'),
    json_extract(elem.value, '$.prompt_path'),
    COALESCE(json_extract(elem.value, '$.interactive'), 0)
FROM projects p, json_each(p.statuses) elem
WHERE p.is_latest = 1;
