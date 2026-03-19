-- GitHub tokens are now stored exclusively in the encrypted user_secrets table.
-- Drop the unused plaintext columns from users_v2.
ALTER TABLE hydra.users_v2 DROP COLUMN github_token;
ALTER TABLE hydra.users_v2 DROP COLUMN github_refresh_token;
