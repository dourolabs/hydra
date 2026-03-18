CREATE TABLE IF NOT EXISTS metis.user_secrets (
    username TEXT NOT NULL,
    secret_name TEXT NOT NULL,
    encrypted_value BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (username, secret_name)
);
