-- PR 1 of the interactive frontend dev preview (i-fvswjkxq).
--
-- Adds a nullable JSONB column on `metis.tasks_v2` recording the proxy
-- targets the worker has advertised via `hydra worker proxy {start,stop}`.
-- Default NULL so existing rows inflate to an empty `Vec<ProxyTarget>` on
-- read.
ALTER TABLE metis.tasks_v2 ADD COLUMN proxy_targets JSONB DEFAULT NULL;
