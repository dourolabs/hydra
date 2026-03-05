-- GitHub tokens are being migrated to encrypted storage in user_secrets.
-- Make these columns nullable so new rows don't require them.
ALTER TABLE metis.users_v2 ALTER COLUMN github_token DROP NOT NULL;
ALTER TABLE metis.users_v2 ALTER COLUMN github_refresh_token DROP NOT NULL;
