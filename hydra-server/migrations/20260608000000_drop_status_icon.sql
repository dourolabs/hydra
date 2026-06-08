-- Strip the now-removed `icon` key from every status declared in
-- `metis.projects.statuses`. The predecessor seed migration
-- (`20260607000000_seed_default_project.sql`) embedded `"icon": "..."`
-- for each of the 5 default-project statuses; this migration brings
-- already-seeded rows in line with the `StatusDefinition` wire type
-- after `IconKey` was removed.
--
-- Idempotent: `elem - 'icon'` is a no-op when the key is absent, so
-- re-running this on rows that already lack `icon` leaves them
-- unchanged.
UPDATE metis.projects
SET statuses = (
    SELECT jsonb_agg(elem - 'icon')
    FROM jsonb_array_elements(statuses) elem
);
