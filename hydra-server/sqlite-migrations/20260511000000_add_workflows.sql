-- Workflow instances (versioned, following the is_latest pattern).
CREATE TABLE IF NOT EXISTS workflows (
    workflow_id TEXT NOT NULL,
    version_number INTEGER NOT NULL,
    template_path TEXT NOT NULL,
    template_snapshot TEXT NOT NULL,
    tracking_issue_id TEXT NOT NULL,
    current_state TEXT NOT NULL,
    context TEXT NOT NULL,
    active_issue_id TEXT,
    history TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    actor TEXT,
    is_latest INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (workflow_id, version_number)
);

CREATE INDEX IF NOT EXISTS idx_workflows_tracking_issue_id
    ON workflows(tracking_issue_id) WHERE is_latest = 1;
CREATE INDEX IF NOT EXISTS idx_workflows_active_issue_id
    ON workflows(active_issue_id) WHERE is_latest = 1;
CREATE INDEX IF NOT EXISTS idx_workflows_status
    ON workflows(status) WHERE is_latest = 1;
CREATE INDEX IF NOT EXISTS idx_workflows_is_latest
    ON workflows(workflow_id) WHERE is_latest = 1;

-- Reverse index mapping child issues back to the workflow that created them.
CREATE TABLE IF NOT EXISTS workflow_issues (
    workflow_id TEXT NOT NULL,
    issue_id TEXT NOT NULL,
    state_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (workflow_id, issue_id)
);

CREATE INDEX IF NOT EXISTS idx_workflow_issues_issue_id
    ON workflow_issues(issue_id);
