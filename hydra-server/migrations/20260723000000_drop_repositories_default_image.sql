-- Drop the dead `metis.repositories_v2.default_image` column. The per-repo
-- image override has never flowed into image resolution at runtime:
-- `ResolvedBundle` was hardcoded to `default_image: None` in
-- `resolve_context`, so the only consumer was unreachable. Image configuration
-- now lives on agents and projects (`session_settings.image`), with the
-- cluster-wide `job.default_image` as the final fallback.
ALTER TABLE metis.repositories_v2
    DROP COLUMN IF EXISTS default_image;
