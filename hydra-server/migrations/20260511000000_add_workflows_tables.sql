-- Add workflows and workflow_issues tables for the workflow engine.
--
-- The workflows table follows the same versioned / is_latest pattern as
-- conversations_v2 (see 20260509000000_add_conversations_tables.sql): rows are
-- never updated in place; each upsert appends a new (id, version_number) row
-- and the maintain_latest_version() trigger flips is_latest to keep exactly
-- one latest row per id.
--
-- workflow_issues is a reverse-lookup index from child issue -> workflow so the
-- engine can resolve `find_workflow_by_issue_id` in a single SQL JOIN.

--------------------------------------------------------------------------------
-- metis.workflows_v2 — workflow instance state (versioned)
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS metis.workflows_v2 (
    id TEXT NOT NULL,
    version_number BIGINT NOT NULL,
    template_path TEXT NOT NULL,
    template_snapshot JSONB NOT NULL,
    tracking_issue_id TEXT NOT NULL,
    current_state TEXT NOT NULL,
    context JSONB NOT NULL DEFAULT '{}'::jsonb,
    active_issue_id TEXT,
    history JSONB NOT NULL DEFAULT '[]'::jsonb,
    status TEXT NOT NULL DEFAULT 'active',
    actor JSONB,
    is_latest BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, version_number)
);

-- Trigger to auto-update updated_at on row update.
DROP TRIGGER IF EXISTS set_timestamp_workflows_v2 ON metis.workflows_v2;
CREATE TRIGGER set_timestamp_workflows_v2
BEFORE UPDATE ON metis.workflows_v2
FOR EACH ROW
EXECUTE FUNCTION metis.touch_updated_at();

-- Trigger to maintain is_latest flag on insert (same shared function used by
-- every other versioned table).
CREATE TRIGGER trg_maintain_latest_workflows_v2
    BEFORE INSERT ON metis.workflows_v2
    FOR EACH ROW EXECUTE FUNCTION metis.maintain_latest_version();

-- Indexes for the filters exposed by WorkflowFilter and for fast id lookups.
CREATE INDEX workflows_v2_latest_id_idx
    ON metis.workflows_v2 (id) WHERE is_latest = true;

CREATE INDEX workflows_v2_latest_tracking_issue_idx
    ON metis.workflows_v2 (tracking_issue_id) WHERE is_latest = true;

CREATE INDEX workflows_v2_latest_active_issue_idx
    ON metis.workflows_v2 (active_issue_id)
    WHERE is_latest = true AND active_issue_id IS NOT NULL;

CREATE INDEX workflows_v2_latest_active_status_idx
    ON metis.workflows_v2 (status) WHERE is_latest = true AND status = 'active';

CREATE INDEX workflows_v2_latest_pagination_idx
    ON metis.workflows_v2 (created_at DESC, id DESC) WHERE is_latest = true;

--------------------------------------------------------------------------------
-- metis.workflow_issues — reverse index: child issue -> workflow
--------------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS metis.workflow_issues (
    workflow_id TEXT NOT NULL,
    issue_id TEXT NOT NULL,
    state_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (workflow_id, issue_id)
);

CREATE INDEX workflow_issues_issue_id_idx
    ON metis.workflow_issues (issue_id);
