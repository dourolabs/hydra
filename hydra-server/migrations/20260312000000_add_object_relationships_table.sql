-- Add object_relationships table for unified relationship storage.
-- Phase 1: create the table, indexes, and backfill from existing JSONB data.

CREATE TABLE IF NOT EXISTS metis.object_relationships (
    source_id    TEXT NOT NULL,
    source_kind  TEXT NOT NULL,
    target_id    TEXT NOT NULL,
    target_kind  TEXT NOT NULL,
    rel_type     TEXT NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (source_id, rel_type, target_id)
);

CREATE INDEX IF NOT EXISTS object_relationships_target_idx
    ON metis.object_relationships (target_id, rel_type);
CREATE INDEX IF NOT EXISTS object_relationships_source_idx
    ON metis.object_relationships (source_id, rel_type);

-- Backfill issue dependencies (child-of, blocked-on) from JSONB
INSERT INTO metis.object_relationships (source_id, source_kind, target_id, target_kind, rel_type)
SELECT DISTINCT ON (i.id, dep->>'issue_id', dep->>'type')
    i.id AS source_id,
    'issue' AS source_kind,
    dep->>'issue_id' AS target_id,
    'issue' AS target_kind,
    dep->>'type' AS rel_type
FROM metis.issues_v2 i,
     LATERAL jsonb_array_elements(i.dependencies) AS dep
WHERE i.version_number = (
    SELECT MAX(version_number) FROM metis.issues_v2 WHERE id = i.id
)
AND NOT i.deleted
AND jsonb_array_length(i.dependencies) > 0
ON CONFLICT DO NOTHING;

-- Backfill issue-patch links from JSONB
INSERT INTO metis.object_relationships (source_id, source_kind, target_id, target_kind, rel_type)
SELECT DISTINCT ON (i.id, patch_id)
    i.id AS source_id,
    'issue' AS source_kind,
    patch_id::TEXT AS target_id,
    'patch' AS target_kind,
    'has-patch' AS rel_type
FROM metis.issues_v2 i,
     LATERAL jsonb_array_elements_text(i.patches) AS patch_id
WHERE i.version_number = (
    SELECT MAX(version_number) FROM metis.issues_v2 WHERE id = i.id
)
AND NOT i.deleted
AND jsonb_array_length(i.patches) > 0
ON CONFLICT DO NOTHING;
