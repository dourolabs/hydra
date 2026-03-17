-- Bump actor payload schema version for required auth token salt.
UPDATE metis.payload_schema_versions
SET current_version = 3
WHERE object_type = 'actor';

-- Backfill auth_token_salt when migrating older actor payloads.
CREATE OR REPLACE FUNCTION metis.migrate_payload(
    object_type TEXT,
    from_version INTEGER,
    to_version INTEGER,
    payload JSONB
) RETURNS JSONB AS $$
DECLARE
    updated JSONB := payload;
    salt TEXT;
BEGIN
    IF object_type = 'actor' AND to_version >= 3 AND from_version < 3 THEN
        IF NOT (updated ? 'auth_token_salt') OR (updated->>'auth_token_salt') IS NULL
            OR (updated->>'auth_token_salt') = '' THEN
            salt := md5(random()::text || clock_timestamp()::text);
            updated := jsonb_set(updated, '{auth_token_salt}', to_jsonb(salt), true);
        END IF;
    END IF;

    RETURN updated;
END;
$$ LANGUAGE plpgsql STABLE;
