-- Rename the PostgreSQL schema from metis to hydra as part of the
-- project-wide rename. This must be applied after all prior migrations
-- (which created objects under the metis schema) and will make the
-- schema name match the new hydra crate and code references.
ALTER SCHEMA metis RENAME TO hydra;
