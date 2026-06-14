-- Rewrite persisted `triggers.schedule` and `triggers.actions` JSON
-- (stored as TEXT) from the legacy externally-tagged PascalCase shape
--   schedule = {"Cron": {"expression": "...", "timezone": "..."}}
--   actions  = [{"CreateIssue": {"type": "task", ...}}]
-- to the new internally-tagged snake_case shape
--   schedule = {"type": "cron", "expression": "...", "timezone": "..."}
--   actions  = [{"type": "create_issue", "issue_type": "task", ...}]
--
-- The `CreateIssue` payload also drops `#[serde(rename = "type")]` on
-- `issue_type`, so the inner `"type"` key is renamed to `"issue_type"` to
-- keep room for the new outer `"type"` discriminator.
--
-- Both UPDATEs are idempotent: rows already carrying the new shape
-- (`schedule.type` is present, every action element has a `type` key)
-- are skipped.

-- ---- schedule -------------------------------------------------------
UPDATE triggers
SET schedule = CASE
    WHEN json_type(schedule, '$.Cron') = 'object'
        THEN json_patch(
            json_object('type', 'cron'),
            json_extract(schedule, '$.Cron')
        )
    WHEN json_type(schedule, '$.Once') = 'object'
        THEN json_patch(
            json_object('type', 'once'),
            json_extract(schedule, '$.Once')
        )
    ELSE schedule
END
WHERE json_type(schedule, '$.type') IS NULL;

-- ---- actions --------------------------------------------------------
-- Walk the JSON array with json_each, rewrite each element, and
-- re-aggregate. Only rows containing at least one un-migrated element
-- (no `type` discriminator) are touched.
--
-- The inner `ORDER BY key` on the json_each subquery mirrors the
-- PG sibling's `WITH ORDINALITY ... ORDER BY ord`. SQLite's
-- json_each emits array elements in index order today and
-- json_group_array preserves the input row order from an ordered
-- subquery, but the explicit ORDER BY makes the asymmetry with the
-- PG version go away and pins ordering contractually.
--
-- The outer `json(...)` wrapper around `rewritten` is load-bearing:
-- SQLite's JSON subtype flag does not propagate through a subquery
-- column alias, so without it `json_group_array` would treat each
-- element as a TEXT string and double-quote it instead of inlining
-- it as JSON.
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
