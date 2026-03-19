-- Migration status table to track v1 to v2 data migration progress.
-- This enables idempotent migration by recording which tables have been migrated.

CREATE TABLE IF NOT EXISTS hydra.migration_status (
    id TEXT PRIMARY KEY,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at TIMESTAMPTZ,
    migrated_count BIGINT NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'in_progress'
);

COMMENT ON TABLE hydra.migration_status IS 'Tracks migration progress from v1 to v2 tables';
COMMENT ON COLUMN hydra.migration_status.id IS 'Migration identifier (e.g., "v1_to_v2_issues")';
COMMENT ON COLUMN hydra.migration_status.status IS 'One of: in_progress, completed, failed';
