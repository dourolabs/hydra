-- list_issues paginates on (updated_at DESC, id DESC), but the prior
-- pagination index was keyed on (created_at DESC, id DESC) and so could
-- never be used by the planner. Swap it for an updated_at-keyed index.
CREATE INDEX IF NOT EXISTS issues_v2_updated_at_id_idx
    ON issues_v2 (updated_at DESC, id DESC);
DROP INDEX IF EXISTS issues_v2_created_at_id_idx;
