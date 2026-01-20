-- Store issue progress as an array of entries (text, timestamp, author) instead of a single string.
-- PostgreSQL syntax.
BEGIN;

ALTER TABLE issues ADD COLUMN progress_entries JSONB NOT NULL DEFAULT '[]'::jsonb;

UPDATE issues
SET progress_entries = CASE
    WHEN progress IS NULL OR length(trim(progress)) = 0 THEN '[]'::jsonb
    ELSE jsonb_build_array(
        jsonb_build_object(
            'text', progress,
            'timestamp', now(),
            'author', NULL
        )
    )
END;

ALTER TABLE issues DROP COLUMN progress;
ALTER TABLE issues RENAME COLUMN progress_entries TO progress;

COMMIT;

-- SQLite fallback (apply manually if needed):
-- ALTER TABLE issues ADD COLUMN progress_entries TEXT NOT NULL DEFAULT '[]';
-- UPDATE issues SET progress_entries = CASE
--     WHEN progress IS NULL OR trim(progress) = '' THEN '[]'
--     ELSE json_array(json_object('text', progress, 'timestamp', strftime('%Y-%m-%dT%H:%M:%fZ','now'), 'author', NULL))
-- END;
-- ALTER TABLE issues DROP COLUMN progress;
-- ALTER TABLE issues RENAME COLUMN progress_entries TO progress;
