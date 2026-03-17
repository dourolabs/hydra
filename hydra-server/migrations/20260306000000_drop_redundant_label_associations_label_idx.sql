-- Drop redundant index: label_associations_label_idx on (label_id) is covered
-- by the primary key (label_id, object_id).
DROP INDEX IF EXISTS hydra.label_associations_label_idx;
