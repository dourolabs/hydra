-- Create `metis.statuses` as the canonical store for per-project status
-- definitions and backfill it from the existing `metis.projects.statuses`
-- JSONB array on every latest-version project row (regardless of
-- `deleted`). See [[i-bqimglba]] for the migration design and
-- `/designs/per-project-issue-statuses.md` for the broader plan.
--
-- PR 1 of 4: pure additive schema; no application code reads or writes
-- the new table yet. The JSONB `projects.statuses` column stays in
-- place as the source of truth through PR 4.
--
-- The composite primary key is `(project_id, sequence)`. The sequence
-- id is the stable storage identity that survives a future
-- `StatusKey` rename — `key` is unique within a project today, but
-- only as a configuration handle; the storage column on
-- `metis.issues_v2` (added in the sibling migration) targets
-- `sequence` so a rename does not orphan issues.
--
-- The `on_enter` column stores the whole `StatusOnEnter` serde shape
-- as a single JSONB blob (NULL when None) for round-trip fidelity at
-- zero schema cost. `color` is the `Rgb` newtype's serialized hex form
-- ("#rrggbb"), stored as TEXT. `interactive` defaults to FALSE on the
-- column for older JSONB rows that may have omitted the field
-- (`#[serde(default)]` on the Rust side).

CREATE TABLE IF NOT EXISTS metis.statuses (
    project_id           TEXT    NOT NULL,
    sequence             BIGINT  NOT NULL,
    key                  TEXT    NOT NULL,
    label                TEXT    NOT NULL,
    color                TEXT    NOT NULL,
    unblocks_parents     BOOLEAN NOT NULL,
    unblocks_dependents  BOOLEAN NOT NULL,
    cascades_to_children BOOLEAN NOT NULL,
    on_enter             JSONB   NULL,
    prompt_path          TEXT    NULL,
    interactive          BOOLEAN NOT NULL DEFAULT FALSE,
    PRIMARY KEY (project_id, sequence)
);

CREATE UNIQUE INDEX IF NOT EXISTS statuses_project_key_idx
    ON metis.statuses (project_id, key);

-- Backfill from the JSONB array on every `is_latest = TRUE` project
-- row. `jsonb_array_elements ... WITH ORDINALITY` yields a 1-indexed
-- `ord` that we use directly as `sequence`. `NULLIF(... 'null'::jsonb)`
-- collapses an explicit JSONB `null` value of `on_enter` to SQL NULL so
-- the column stays an Option<StatusOnEnter> at the Rust boundary in
-- later PRs. `ON CONFLICT (project_id, sequence) DO NOTHING` makes the
-- statement idempotent if the body is ever re-applied outside the
-- migration tracker.
INSERT INTO metis.statuses (
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
    elem.ord,
    elem.value->>'key',
    elem.value->>'label',
    elem.value->>'color',
    (elem.value->>'unblocks_parents')::boolean,
    (elem.value->>'unblocks_dependents')::boolean,
    (elem.value->>'cascades_to_children')::boolean,
    NULLIF(elem.value->'on_enter', 'null'::jsonb),
    elem.value->>'prompt_path',
    COALESCE((elem.value->>'interactive')::boolean, FALSE)
FROM metis.projects p,
     jsonb_array_elements(p.statuses) WITH ORDINALITY AS elem(value, ord)
WHERE p.is_latest = TRUE
ON CONFLICT (project_id, sequence) DO NOTHING;
