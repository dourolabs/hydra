import { useState } from "react";
import { Panel, Spinner, Button } from "@hydra/ui";
import type { RepositoryRecord } from "@hydra/api";
import { useRepositories } from "../../hooks/useRepositories";
import { RepositoryRow } from "./RepositoryRow";
import { RepositoryCreateModal } from "./RepositoryCreateModal";
import { RepositoryEditModal } from "./RepositoryEditModal";
import { RepositoryDeleteModal } from "./RepositoryDeleteModal";
import sharedStyles from "../../components/SettingsSection/SettingsSection.module.css";

export function RepositoriesSection() {
  const { data: repositories, isLoading, error } = useRepositories();
  const [createOpen, setCreateOpen] = useState(false);
  const [editTarget, setEditTarget] = useState<RepositoryRecord | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<RepositoryRecord | null>(null);

  return (
    <>
      {isLoading && (
        <div className={sharedStyles.center}>
          <Spinner size="md" />
        </div>
      )}

      {error && (
        <p className={sharedStyles.error}>Failed to load repositories: {(error as Error).message}</p>
      )}

      <Panel
        header={
          <div className={sharedStyles.panelHeaderRow}>
            <span className={sharedStyles.sectionTitle}>Repositories</span>
            <Button variant="primary" size="sm" onClick={() => setCreateOpen(true)}>
              Add Repository
            </Button>
          </div>
        }
      >
        {repositories && repositories.length === 0 && (
          <p className={sharedStyles.empty}>No repositories configured.</p>
        )}
        {repositories && repositories.length > 0 && (
          <div className={sharedStyles.itemList}>
            {repositories.map((repo) => (
              <RepositoryRow
                key={repo.name}
                repo={repo}
                onEdit={() => setEditTarget(repo)}
                onDelete={() => setDeleteTarget(repo)}
              />
            ))}
          </div>
        )}
      </Panel>

      <RepositoryCreateModal open={createOpen} onClose={() => setCreateOpen(false)} />

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
