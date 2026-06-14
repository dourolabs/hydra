-- Tighten action-array ordering in the trigger rewrite migration.
--
-- The companion 20260722000000 migration rewrites the legacy
-- externally-tagged `triggers.actions` array into the
-- internally-tagged shape, walking the array with `json_each` and
-- re-aggregating with `json_group_array`. That migration relies on
-- SQLite's de-facto `json_each` row-emission order and
-- `json_group_array`-input-row-order preservation. Both are correct
-- in current SQLite versions but not contractually guaranteed; the
-- PG sibling pins this with `WITH ORDINALITY ... ORDER BY ord`.
--
-- This migration re-applies the same rewrite with an explicit
-- `ORDER BY key` over the `json_each(actions)` source row stream and
-- wraps the projected element in `json(...)` so the JSON subtype flag
-- survives the subquery alias (without it, `json_group_array` treats
-- the value as TEXT and double-quotes it).
--
-- On databases that already ran 20260722000000 successfully (every
-- action element carries the new `type` discriminator), the `EXISTS`
-- guard skips every row and this migration is a no-op. On any
-- database where 20260722000000 was interrupted mid-flight and
-- legacy-shape rows survived, this migration finishes the conversion
-- with explicit ordering. Either way the contractual tightening is
-- recorded in the migration history.

UPDATE triggers
SET actions = (
    SELECT json_group_array(json(rewritten))
    FROM (
        SELECT
            CASE
                WHEN json_type(value, '$.CreateIssue') = 'object'
                    THEN json_patch(
                        json_object('type', 'create_issue'),
                        json_remove(
                            json_set(
                                json_extract(value, '$.CreateIssue'),
                                '$.issue_type', json_extract(value, '$.CreateIssue.type')
                            ),
                            '$.type'
                        )
                    )
                ELSE json(value)
            END AS rewritten
        FROM json_each(actions)
        ORDER BY key
    )
)
WHERE actions IS NOT NULL
  AND EXISTS (
      SELECT 1 FROM json_each(triggers.actions)
      WHERE json_type(value, '$.type') IS NULL
  );
