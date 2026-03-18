-- Add versioned storage for documents and supporting indexes.
INSERT INTO metis.payload_schema_versions (object_type, current_version)
VALUES ('document', 1)
ON CONFLICT (object_type) DO NOTHING;

CREATE TABLE IF NOT EXISTS metis.documents (
    id TEXT NOT NULL,
    version_number BIGINT NOT NULL DEFAULT 1,
    schema_version INTEGER NOT NULL DEFAULT metis.current_schema_version('document'),
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (schema_version > 0),
    CHECK (version_number > 0),
    PRIMARY KEY (id, version_number)
);

CREATE INDEX IF NOT EXISTS documents_latest_idx
    ON metis.documents (id, version_number DESC);

CREATE INDEX IF NOT EXISTS documents_path_idx
    ON metis.documents ((payload->>'path'));

CREATE INDEX IF NOT EXISTS documents_path_prefix_idx
    ON metis.documents USING btree ((payload->>'path') text_pattern_ops);

CREATE INDEX IF NOT EXISTS documents_created_by_idx
    ON metis.documents ((payload->>'created_by'));

DROP TRIGGER IF EXISTS set_timestamp_documents ON metis.documents;
CREATE TRIGGER set_timestamp_documents
BEFORE UPDATE ON metis.documents
FOR EACH ROW
EXECUTE FUNCTION metis.touch_updated_at();
