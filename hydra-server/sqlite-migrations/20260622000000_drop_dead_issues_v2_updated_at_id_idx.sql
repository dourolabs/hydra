-- `issues_v2_updated_at_id_idx` was added by 20260605000000 when
-- `list_issues` paginated on `updated_at DESC`. The keyset anchor was
-- swung back to `created_at DESC` (see p-kzbakldw), which the partial
-- `issues_v2_latest_pagination_idx` from 20260318000000 already covers.
-- The `updated_at`-keyed index is now pure write amplification on every
-- issue insert/update -- drop it. The sqlite table-rebuild migrations
-- 20260612000000 and 20260614000000 each recreated this index as part
-- of their rebuilds, so this drop must run after them.
DROP INDEX IF EXISTS issues_v2_updated_at_id_idx;
