import { useState } from "react";
import { Panel, Spinner, Button } from "@metis/ui";
import type { RepositoryRecord } from "@metis/api";
import { useRepositories } from "../../hooks/useRepositories";
import { RepositoryRow } from "./RepositoryRow";
import { RepositoryCreateModal } from "./RepositoryCreateModal";
import { RepositoryEditModal } from "./RepositoryEditModal";
import { RepositoryDeleteModal } from "./RepositoryDeleteModal";
import styles from "./RepositoriesSection.module.css";

export function RepositoriesSection() {
  const { data: repositories, isLoading, error } = useRepositories();
  const [createOpen, setCreateOpen] = useState(false);
  const [editTarget, setEditTarget] = useState<RepositoryRecord | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<RepositoryRecord | null>(
    null,
  );

  return (
    <>
      <div className={styles.headerRow}>
        <Button variant="primary" size="sm" onClick={() => setCreateOpen(true)}>
          Add Repository
        </Button>
      </div>

      {isLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}

      {error && (
        <p className={styles.error}>
          Failed to load repositories: {(error as Error).message}
        </p>
      )}

      {repositories && repositories.length === 0 && (
        <p className={styles.empty}>No repositories configured.</p>
      )}

      {repositories && repositories.length > 0 && (
        <Panel
          header={
            <span className={styles.sectionTitle}>Repositories</span>
          }
        >
          <div className={styles.repoList}>
            {repositories.map((repo) => (
              <RepositoryRow
                key={repo.name}
                repo={repo}
                onEdit={() => setEditTarget(repo)}
                onDelete={() => setDeleteTarget(repo)}
              />
            ))}
          </div>
        </Panel>
      )}

      <RepositoryCreateModal
        open={createOpen}
        onClose={() => setCreateOpen(false)}
      />

      {editTarget && (
        <RepositoryEditModal
          open={!!editTarget}
          repo={editTarget}
          onClose={() => setEditTarget(null)}
        />
      )}

      {deleteTarget && (
        <RepositoryDeleteModal
          open={!!deleteTarget}
          repo={deleteTarget}
          onClose={() => setDeleteTarget(null)}
        />
      )}
    </>
  );
}
