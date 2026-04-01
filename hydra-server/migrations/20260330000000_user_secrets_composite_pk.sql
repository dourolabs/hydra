-- Change primary key to (username, secret_name, internal) so both an internal
-- and external version of the same secret can coexist.
ALTER TABLE metis.user_secrets DROP CONSTRAINT user_secrets_pkey;
ALTER TABLE metis.user_secrets ADD PRIMARY KEY (username, secret_name, internal);
