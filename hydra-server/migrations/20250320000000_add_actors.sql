-- Add actor payload support.
INSERT INTO metis.payload_schema_versions (object_type, current_version)
VALUES ('actor', 1)
ON CONFLICT (object_type) DO NOTHING;

CREATE TABLE IF NOT EXISTS metis.actors (
    id TEXT PRIMARY KEY,
    schema_version INTEGER NOT NULL DEFAULT metis.current_schema_version('actor'),
    payload JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (schema_version > 0)
);

DROP TRIGGER IF EXISTS set_timestamp_actors ON metis.actors;
CREATE TRIGGER set_timestamp_actors
BEFORE UPDATE ON metis.actors
FOR EACH ROW
EXECUTE FUNCTION metis.touch_updated_at();
