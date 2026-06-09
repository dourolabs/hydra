-- baseline-version: 20260614000000
-- Postgres pre-reserve-hydra-id-shape baseline. INSERTs are valid
-- against the schema state after sqlx migration
-- `20260614000000_cutover_to_statuses_table.sql` and immediately
-- before `20260615000000_reserve_hydra_id_shape_in_keys.sql`
-- rewrites the shape-matching keys. See [[i-njohfbbk]] for design.
--
-- Coverage exercised by these rows (asserted in
-- `migration_roundtrip.rs::reserve_hydra_id_shape_*`):
--
--   * `j-rsvshapa` (key `j-foo`): shape-matching project key — must
--     be rewritten to `renamed-j-foo`.
--   * `j-rsvshapb` (key `engineering`): safe project key — must be
--     untouched.
--   * `j-rsvshapc` (key `renamed-x-old`): pre-existing literal that
--     happens to begin with `renamed-` — the idempotency probe. Does
--     not match the reserved shape; must be untouched.
--
-- Status rows on `j-rsvshapa`:
--   * seq=1 key `i-progress`: shape-matching, no collision → rewrite
--     to `renamed-i-progress`.
--   * seq=2 key `done`: safe — untouched.
--   * seq=3 key `s-todo`: shape-matching, collides with the literal
--     `renamed-s-todo` at seq=4 → rewrite to `renamed-s-todo-seq3`
--     via the `(project_id, key)` collision-disambiguator branch.
--   * seq=4 key `renamed-s-todo`: pre-existing literal collision
--     target — untouched.

-- Shape-matching project key: must be rewritten.
INSERT INTO metis.projects (
    id,
    version_number,
    key,
    name,
    creator,
    prompt_path,
    next_status_sequence
)
VALUES (
    'j-rsvshapa',
    1,
    'j-foo',
    'Reserve Shape - Shape Match',
    'jayantk',
    NULL,
    5
);

-- Safe project key: must be untouched.
INSERT INTO metis.projects (
    id,
    version_number,
    key,
    name,
    creator,
    prompt_path,
    next_status_sequence
)
VALUES (
    'j-rsvshapb',
    1,
    'engineering',
    'Reserve Shape - Safe Key',
    'jayantk',
    NULL,
    1
);

-- Idempotency probe: literal already in the `renamed-` form. Does
-- not match `[a-z]-...` (the second byte is `e`, not `-`), so the
-- new migration must leave it alone.
INSERT INTO metis.projects (
    id,
    version_number,
    key,
    name,
    creator,
    prompt_path,
    next_status_sequence
)
VALUES (
    'j-rsvshapc',
    1,
    'renamed-x-old',
    'Reserve Shape - Idempotency Probe',
    'jayantk',
    NULL,
    1
);

-- Statuses on the shape-matching project. Exercises four states:
-- shape-match-rewrite, safe-untouched, shape-match-with-collision,
-- and pre-existing literal collision target.
INSERT INTO metis.statuses (
    project_id, sequence, key, label, color,
    unblocks_parents, unblocks_dependents, cascades_to_children,
    on_enter, prompt_path, interactive
) VALUES
    ('j-rsvshapa', 1, 'i-progress',    'In progress', '#3498db', FALSE, FALSE, FALSE, NULL, NULL, FALSE),
    ('j-rsvshapa', 2, 'done',          'Done',        '#2ecc71', TRUE,  TRUE,  FALSE, NULL, NULL, FALSE),
    ('j-rsvshapa', 3, 's-todo',        'Todo',        '#aaaaaa', FALSE, FALSE, FALSE, NULL, NULL, FALSE),
    ('j-rsvshapa', 4, 'renamed-s-todo', 'Renamed Todo', '#bbbbbb', FALSE, FALSE, FALSE, NULL, NULL, FALSE);
