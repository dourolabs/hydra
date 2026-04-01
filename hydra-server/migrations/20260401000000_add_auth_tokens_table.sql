-- Add auth_tokens table for multi-token storage per actor.
-- Copies existing token hashes from actors_v2 into the new table.

CREATE TABLE IF NOT EXISTS metis.auth_tokens (
    actor_name TEXT NOT NULL,
    token_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (actor_name, token_hash)
);

CREATE INDEX IF NOT EXISTS auth_tokens_actor_name_idx ON metis.auth_tokens (actor_name);

-- Migrate existing token hashes from the latest version of each actor.
INSERT INTO metis.auth_tokens (actor_name, token_hash, created_at)
SELECT DISTINCT ON (a.id) a.id, a.auth_token_hash, a.created_at
FROM metis.actors_v2 a
WHERE a.auth_token_hash != ''
ORDER BY a.id, a.version_number DESC
ON CONFLICT DO NOTHING;
