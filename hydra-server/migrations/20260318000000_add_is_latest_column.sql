-- Add is_latest BOOLEAN column to all versioned tables and maintain it via
-- a BEFORE INSERT trigger so that list/count queries can filter on
-- WHERE is_latest = true instead of correlated subqueries.

-- 1. Add the column to all 8 versioned tables
ALTER TABLE metis.issues_v2       ADD COLUMN is_latest BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE metis.patches_v2      ADD COLUMN is_latest BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE metis.tasks_v2        ADD COLUMN is_latest BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE metis.documents_v2    ADD COLUMN is_latest BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE metis.users_v2        ADD COLUMN is_latest BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE metis.actors_v2       ADD COLUMN is_latest BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE metis.repositories_v2 ADD COLUMN is_latest BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE metis.messages_v2     ADD COLUMN is_latest BOOLEAN NOT NULL DEFAULT FALSE;

-- 2. Create a single trigger function that works for all tables via
--    TG_TABLE_SCHEMA / TG_TABLE_NAME dynamic references.
CREATE OR REPLACE FUNCTION metis.maintain_latest_version() RETURNS TRIGGER AS $$
BEGIN
    EXECUTE format(
        'UPDATE %I.%I SET is_latest = false WHERE id = $1 AND is_latest = true',
        TG_TABLE_SCHEMA, TG_TABLE_NAME
    ) USING NEW.id;
    NEW.is_latest := true;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- 3. Attach BEFORE INSERT triggers to all 8 tables
CREATE TRIGGER trg_maintain_latest_issues_v2
    BEFORE INSERT ON metis.issues_v2
    FOR EACH ROW EXECUTE FUNCTION metis.maintain_latest_version();

CREATE TRIGGER trg_maintain_latest_patches_v2
    BEFORE INSERT ON metis.patches_v2
    FOR EACH ROW EXECUTE FUNCTION metis.maintain_latest_version();

CREATE TRIGGER trg_maintain_latest_tasks_v2
    BEFORE INSERT ON metis.tasks_v2
    FOR EACH ROW EXECUTE FUNCTION metis.maintain_latest_version();

CREATE TRIGGER trg_maintain_latest_documents_v2
    BEFORE INSERT ON metis.documents_v2
    FOR EACH ROW EXECUTE FUNCTION metis.maintain_latest_version();

CREATE TRIGGER trg_maintain_latest_users_v2
    BEFORE INSERT ON metis.users_v2
    FOR EACH ROW EXECUTE FUNCTION metis.maintain_latest_version();

CREATE TRIGGER trg_maintain_latest_actors_v2
    BEFORE INSERT ON metis.actors_v2
    FOR EACH ROW EXECUTE FUNCTION metis.maintain_latest_version();

CREATE TRIGGER trg_maintain_latest_repositories_v2
    BEFORE INSERT ON metis.repositories_v2
    FOR EACH ROW EXECUTE FUNCTION metis.maintain_latest_version();

CREATE TRIGGER trg_maintain_latest_messages_v2
    BEFORE INSERT ON metis.messages_v2
    FOR EACH ROW EXECUTE FUNCTION metis.maintain_latest_version();

-- 4. Backfill is_latest = true for existing latest-version rows
UPDATE metis.issues_v2 SET is_latest = true
    WHERE (id, version_number) IN (SELECT id, MAX(version_number) FROM metis.issues_v2 GROUP BY id);

UPDATE metis.patches_v2 SET is_latest = true
    WHERE (id, version_number) IN (SELECT id, MAX(version_number) FROM metis.patches_v2 GROUP BY id);

UPDATE metis.tasks_v2 SET is_latest = true
    WHERE (id, version_number) IN (SELECT id, MAX(version_number) FROM metis.tasks_v2 GROUP BY id);

UPDATE metis.documents_v2 SET is_latest = true
    WHERE (id, version_number) IN (SELECT id, MAX(version_number) FROM metis.documents_v2 GROUP BY id);

UPDATE metis.users_v2 SET is_latest = true
    WHERE (id, version_number) IN (SELECT id, MAX(version_number) FROM metis.users_v2 GROUP BY id);

UPDATE metis.actors_v2 SET is_latest = true
    WHERE (id, version_number) IN (SELECT id, MAX(version_number) FROM metis.actors_v2 GROUP BY id);

UPDATE metis.repositories_v2 SET is_latest = true
    WHERE (id, version_number) IN (SELECT id, MAX(version_number) FROM metis.repositories_v2 GROUP BY id);

UPDATE metis.messages_v2 SET is_latest = true
    WHERE (id, version_number) IN (SELECT id, MAX(version_number) FROM metis.messages_v2 GROUP BY id);

-- 5. Create partial indexes for efficient pagination queries
CREATE INDEX issues_v2_latest_pagination_idx       ON metis.issues_v2       (created_at DESC, id DESC) WHERE is_latest = true;
CREATE INDEX patches_v2_latest_pagination_idx      ON metis.patches_v2      (created_at DESC, id DESC) WHERE is_latest = true;
CREATE INDEX tasks_v2_latest_pagination_idx        ON metis.tasks_v2        (created_at DESC, id DESC) WHERE is_latest = true;
CREATE INDEX documents_v2_latest_pagination_idx    ON metis.documents_v2    (created_at DESC, id DESC) WHERE is_latest = true;
CREATE INDEX users_v2_latest_pagination_idx        ON metis.users_v2        (created_at DESC, id DESC) WHERE is_latest = true;
CREATE INDEX actors_v2_latest_pagination_idx       ON metis.actors_v2       (created_at DESC, id DESC) WHERE is_latest = true;
CREATE INDEX repositories_v2_latest_pagination_idx ON metis.repositories_v2 (created_at DESC, id DESC) WHERE is_latest = true;
CREATE INDEX messages_v2_latest_pagination_idx     ON metis.messages_v2     (created_at DESC, id DESC) WHERE is_latest = true;

-- 6. Create partial indexes for efficient lookups by id
CREATE INDEX issues_v2_latest_id_idx       ON metis.issues_v2       (id) WHERE is_latest = true;
CREATE INDEX patches_v2_latest_id_idx      ON metis.patches_v2      (id) WHERE is_latest = true;
CREATE INDEX tasks_v2_latest_id_idx        ON metis.tasks_v2        (id) WHERE is_latest = true;
CREATE INDEX documents_v2_latest_id_idx    ON metis.documents_v2    (id) WHERE is_latest = true;
CREATE INDEX users_v2_latest_id_idx        ON metis.users_v2        (id) WHERE is_latest = true;
CREATE INDEX actors_v2_latest_id_idx       ON metis.actors_v2       (id) WHERE is_latest = true;
CREATE INDEX repositories_v2_latest_id_idx ON metis.repositories_v2 (id) WHERE is_latest = true;
CREATE INDEX messages_v2_latest_id_idx     ON metis.messages_v2     (id) WHERE is_latest = true;
