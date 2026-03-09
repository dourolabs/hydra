-- Add object_relationships table for unified relationship storage.

CREATE TABLE IF NOT EXISTS object_relationships (
    source_id    TEXT NOT NULL,
    source_kind  TEXT NOT NULL,
    target_id    TEXT NOT NULL,
    target_kind  TEXT NOT NULL,
    rel_type     TEXT NOT NULL,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%f+00:00', 'now')),
    PRIMARY KEY (source_id, rel_type, target_id)
);

CREATE INDEX IF NOT EXISTS object_relationships_target_idx
    ON object_relationships (target_id, rel_type);
CREATE INDEX IF NOT EXISTS object_relationships_source_idx
    ON object_relationships (source_id, rel_type);

-- Backfill issue dependencies from JSON
INSERT OR IGNORE INTO object_relationships (source_id, source_kind, target_id, target_kind, rel_type)
SELECT
    i.id AS source_id,
    'issue' AS source_kind,
    json_extract(dep.value, '$.issue_id') AS target_id,
    'issue' AS target_kind,
    json_extract(dep.value, '$.type') AS rel_type
FROM issues_v2 i,
     json_each(i.dependencies) AS dep
WHERE i.version_number = (
    SELECT MAX(version_number) FROM issues_v2 WHERE id = i.id
)
AND i.deleted = 0
AND json_array_length(i.dependencies) > 0;

-- Backfill issue-patch links from JSON
INSERT OR IGNORE INTO object_relationships (source_id, source_kind, target_id, target_kind, rel_type)
SELECT
    i.id AS source_id,
    'issue' AS source_kind,
    patch.value AS target_id,
    'patch' AS target_kind,
    'has-patch' AS rel_type
FROM issues_v2 i,
     json_each(i.patches) AS patch
WHERE i.version_number = (
    SELECT MAX(version_number) FROM issues_v2 WHERE id = i.id
)
AND i.deleted = 0
AND json_array_length(i.patches) > 0;
