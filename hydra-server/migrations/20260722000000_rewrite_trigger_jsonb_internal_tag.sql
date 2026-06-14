-- Rewrite persisted `metis.triggers.schedule` and `metis.triggers.actions`
-- JSONB from the legacy externally-tagged PascalCase shape
--   schedule = {"Cron": {"expression": "...", "timezone": "..."}}
--   actions  = [{"CreateIssue": {"type": "task", ...}}]
-- to the new internally-tagged snake_case shape
--   schedule = {"type": "cron", "expression": "...", "timezone": "..."}
--   actions  = [{"type": "create_issue", "issue_type": "task", ...}]
--
-- The `CreateIssue` payload also drops the `#[serde(rename = "type")]` on
-- `issue_type`, so its inner `"type"` key is renamed to `"issue_type"` to
-- keep room for the new outer `"type"` discriminator (the dominant
-- internally-tagged-union convention in this codebase).
--
-- Both UPDATEs are idempotent: rows already carrying the new shape
-- (`schedule->>'type' IS NOT NULL`, every action element has a `type`
-- key) are skipped.

BEGIN;

UPDATE metis.triggers
SET schedule = CASE
    WHEN schedule ? 'Cron' THEN
        jsonb_build_object('type', 'cron') || (schedule->'Cron')
    WHEN schedule ? 'Once' THEN
        jsonb_build_object('type', 'once') || (schedule->'Once')
    ELSE schedule
END
WHERE NOT (schedule ? 'type');

UPDATE metis.triggers
SET actions = COALESCE(
    (
        SELECT jsonb_agg(
            CASE
                WHEN elem ? 'CreateIssue' THEN
                    jsonb_build_object('type', 'create_issue')
                    || ((elem->'CreateIssue') - 'type')
                    || jsonb_build_object('issue_type', elem->'CreateIssue'->'type')
                ELSE elem
            END
            ORDER BY ord
        )
        FROM jsonb_array_elements(actions) WITH ORDINALITY AS t(elem, ord)
    ),
    '[]'::jsonb
)
WHERE EXISTS (
    SELECT 1
    FROM jsonb_array_elements(actions) AS elem
    WHERE NOT (elem ? 'type')
);

COMMIT;
