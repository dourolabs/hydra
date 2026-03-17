-- Label associations join table: links labels to issues, patches, and documents.
-- object_kind is stored for query performance but can be inferred from the object_id prefix.
CREATE TABLE IF NOT EXISTS hydra.label_associations (
    label_id TEXT NOT NULL REFERENCES hydra.labels(id),
    object_id TEXT NOT NULL,
    object_kind TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (label_id, object_id)
);

CREATE INDEX IF NOT EXISTS label_associations_object_idx
    ON hydra.label_associations (object_id);
CREATE INDEX IF NOT EXISTS label_associations_label_idx
    ON hydra.label_associations (label_id);
