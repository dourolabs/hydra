-- Strip the now-removed `icon` key from every status declared in
-- `projects.statuses`. The predecessor seed migration
-- (`20260607000000_seed_default_project.sql`) embedded `"icon": "..."`
-- for each of the 5 default-project statuses; this migration brings
-- already-seeded rows in line with the `StatusDefinition` wire type
-- after `IconKey` was removed.
--
-- Idempotent: `json_remove` is a no-op when the targeted path is
-- absent, so re-running this on rows that already lack `icon` leaves
-- them unchanged.
UPDATE projects
SET statuses = (
    SELECT json_group_array(json_remove(value, '$.icon'))
    FROM json_each(statuses)
);
