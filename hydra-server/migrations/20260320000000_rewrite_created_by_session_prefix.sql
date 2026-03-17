-- Rewrite created_by on patches and documents from legacy t- session IDs to s- prefix.
-- Matches the session ID migration (20260316000000); these columns were missed.
-- Idempotent: rows where created_by already has s- are not matched.

UPDATE hydra.patches_v2 SET created_by = 's-' || SUBSTRING(created_by FROM 3) WHERE created_by LIKE 't-%';
UPDATE hydra.documents_v2 SET created_by = 's-' || SUBSTRING(created_by FROM 3) WHERE created_by LIKE 't-%';
