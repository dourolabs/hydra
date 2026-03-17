-- Add is_latest BOOLEAN column to all versioned tables and maintain it via
-- a BEFORE INSERT trigger so that list/count queries can filter on
-- WHERE is_latest = true instead of correlated subqueries.

-- 1. Add the column to all 8 versioned tables
ALTER TABLE hydra.issues_v2       ADD COLUMN is_latest BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE hydra.patches_v2      ADD COLUMN is_latest BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE hydra.tasks_v2        ADD COLUMN is_latest BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE hydra.documents_v2    ADD COLUMN is_latest BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE hydra.users_v2        ADD COLUMN is_latest BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE hydra.actors_v2       ADD COLUMN is_latest BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE hydra.repositories_v2 ADD COLUMN is_latest BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE hydra.messages_v2     ADD COLUMN is_latest BOOLEAN NOT NULL DEFAULT FALSE;

-- 2. Create a single trigger function that works for all tables via
--    TG_TABLE_SCHEMA / TG_TABLE_NAME dynamic references.
CREATE OR REPLACE FUNCTION hydra.maintain_latest_version() RETURNS TRIGGER AS $$
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
    BEFORE INSERT ON hydra.issues_v2
    FOR EACH ROW EXECUTE FUNCTION hydra.maintain_latest_version();

CREATE TRIGGER trg_maintain_latest_patches_v2
    BEFORE INSERT ON hydra.patches_v2
    FOR EACH ROW EXECUTE FUNCTION hydra.maintain_latest_version();

CREATE TRIGGER trg_maintain_latest_tasks_v2
    BEFORE INSERT ON hydra.tasks_v2
    FOR EACH ROW EXECUTE FUNCTION hydra.maintain_latest_version();

CREATE TRIGGER trg_maintain_latest_documents_v2
    BEFORE INSERT ON hydra.documents_v2
    FOR EACH ROW EXECUTE FUNCTION hydra.maintain_latest_version();

CREATE TRIGGER trg_maintain_latest_users_v2
    BEFORE INSERT ON hydra.users_v2
    FOR EACH ROW EXECUTE FUNCTION hydra.maintain_latest_version();

CREATE TRIGGER trg_maintain_latest_actors_v2
    BEFORE INSERT ON hydra.actors_v2
    FOR EACH ROW EXECUTE FUNCTION hydra.maintain_latest_version();

CREATE TRIGGER trg_maintain_latest_repositories_v2
    BEFORE INSERT ON hydra.repositories_v2
    FOR EACH ROW EXECUTE FUNCTION hydra.maintain_latest_version();

CREATE TRIGGER trg_maintain_latest_messages_v2
    BEFORE INSERT ON hydra.messages_v2
    FOR EACH ROW EXECUTE FUNCTION hydra.maintain_latest_version();

-- 4. Backfill is_latest = true for existing latest-version rows
UPDATE hydra.issues_v2 SET is_latest = true
    WHERE (id, version_number) IN (SELECT id, MAX(version_number) FROM hydra.issues_v2 GROUP BY id);

UPDATE hydra.patches_v2 SET is_latest = true
    WHERE (id, version_number) IN (SELECT id, MAX(version_number) FROM hydra.patches_v2 GROUP BY id);

UPDATE hydra.tasks_v2 SET is_latest = true
    WHERE (id, version_number) IN (SELECT id, MAX(version_number) FROM hydra.tasks_v2 GROUP BY id);

UPDATE hydra.documents_v2 SET is_latest = true
    WHERE (id, version_number) IN (SELECT id, MAX(version_number) FROM hydra.documents_v2 GROUP BY id);

UPDATE hydra.users_v2 SET is_latest = true
    WHERE (id, version_number) IN (SELECT id, MAX(version_number) FROM hydra.users_v2 GROUP BY id);

UPDATE hydra.actors_v2 SET is_latest = true
    WHERE (id, version_number) IN (SELECT id, MAX(version_number) FROM hydra.actors_v2 GROUP BY id);

UPDATE hydra.repositories_v2 SET is_latest = true
    WHERE (id, version_number) IN (SELECT id, MAX(version_number) FROM hydra.repositories_v2 GROUP BY id);

UPDATE hydra.messages_v2 SET is_latest = true
    WHERE (id, version_number) IN (SELECT id, MAX(version_number) FROM hydra.messages_v2 GROUP BY id);

-- 5. Create partial indexes for efficient pagination queries
CREATE INDEX issues_v2_latest_pagination_idx       ON hydra.issues_v2       (created_at DESC, id DESC) WHERE is_latest = true;
CREATE INDEX patches_v2_latest_pagination_idx      ON hydra.patches_v2      (created_at DESC, id DESC) WHERE is_latest = true;
CREATE INDEX tasks_v2_latest_pagination_idx        ON hydra.tasks_v2        (created_at DESC, id DESC) WHERE is_latest = true;
CREATE INDEX documents_v2_latest_pagination_idx    ON hydra.documents_v2    (created_at DESC, id DESC) WHERE is_latest = true;
CREATE INDEX users_v2_latest_pagination_idx        ON hydra.users_v2        (created_at DESC, id DESC) WHERE is_latest = true;
CREATE INDEX actors_v2_latest_pagination_idx       ON hydra.actors_v2       (created_at DESC, id DESC) WHERE is_latest = true;
CREATE INDEX repositories_v2_latest_pagination_idx ON hydra.repositories_v2 (created_at DESC, id DESC) WHERE is_latest = true;
CREATE INDEX messages_v2_latest_pagination_idx     ON hydra.messages_v2     (created_at DESC, id DESC) WHERE is_latest = true;

-- 6. Create partial indexes for efficient lookups by id
CREATE INDEX issues_v2_latest_id_idx       ON hydra.issues_v2       (id) WHERE is_latest = true;
CREATE INDEX patches_v2_latest_id_idx      ON hydra.patches_v2      (id) WHERE is_latest = true;
CREATE INDEX tasks_v2_latest_id_idx        ON hydra.tasks_v2        (id) WHERE is_latest = true;
CREATE INDEX documents_v2_latest_id_idx    ON hydra.documents_v2    (id) WHERE is_latest = true;
CREATE INDEX users_v2_latest_id_idx        ON hydra.users_v2        (id) WHERE is_latest = true;
CREATE INDEX actors_v2_latest_id_idx       ON hydra.actors_v2       (id) WHERE is_latest = true;
CREATE INDEX repositories_v2_latest_id_idx ON hydra.repositories_v2 (id) WHERE is_latest = true;
CREATE INDEX messages_v2_latest_id_idx     ON hydra.messages_v2     (id) WHERE is_latest = true;
