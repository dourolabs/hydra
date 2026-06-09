-- baseline-version: 20260614000000
-- SQLite pre-reserve-hydra-id-shape baseline. Sister to the
-- postgres `20260614000000__pre_reserve_hydra_id_shape.sql`. INSERTs
-- are valid against the schema state after sqlx migration
-- `20260614000000_cutover_to_statuses_table.sql` and immediately
-- before `20260615000000_reserve_hydra_id_shape_in_keys.sql`.
-- See [[i-njohfbbk]] for design.
--
-- Same coverage matrix as the postgres sibling; rows kept
-- independent so backend-specific fixture changes don't ripple.
-- SQLite has no BEFORE-INSERT trigger to set `is_latest`, so it's
-- passed explicitly.

INSERT INTO projects (
    id, version_number, key, name, creator,
    deleted, actor, prompt_path, is_latest, next_status_sequence
)
VALUES
    -- Shape-matching project key: must be rewritten.
    ('j-rsvshapa', 1, 'j-foo',         'Reserve Shape - Shape Match',         'jayantk', 0, NULL, NULL, 1, 5),
    -- Safe project key: must be untouched.
    ('j-rsvshapb', 1, 'engineering',   'Reserve Shape - Safe Key',            'jayantk', 0, NULL, NULL, 1, 1),
    -- Idempotency probe: literal already in the `renamed-` form.
    ('j-rsvshapc', 1, 'renamed-x-old', 'Reserve Shape - Idempotency Probe',   'jayantk', 0, NULL, NULL, 1, 1);

INSERT INTO statuses (
    project_id, sequence, key, label, color,
    unblocks_parents, unblocks_dependents, cascades_to_children,
    on_enter, prompt_path, interactive
) VALUES
    ('j-rsvshapa', 1, 'i-progress',     'In progress',  '#3498db', 0, 0, 0, NULL, NULL, 0),
    ('j-rsvshapa', 2, 'done',           'Done',         '#2ecc71', 1, 1, 0, NULL, NULL, 0),
    ('j-rsvshapa', 3, 's-todo',         'Todo',         '#aaaaaa', 0, 0, 0, NULL, NULL, 0),
    ('j-rsvshapa', 4, 'renamed-s-todo', 'Renamed Todo', '#bbbbbb', 0, 0, 0, NULL, NULL, 0);
