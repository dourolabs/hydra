import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Panel, Spinner, Button } from "@hydra/ui";
import type { RepositoryRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useRepositories } from "../../hooks/useRepositories";
import { useToast } from "../toast/useToast";
import { RepositoryRow } from "./RepositoryRow";
import { RepositoryCreateModal } from "./RepositoryCreateModal";
import { RepositoryEditModal } from "./RepositoryEditModal";
import { DeleteConfirmModal } from "../../components/DeleteConfirmModal/DeleteConfirmModal";
import styles from "./RepositoriesSection.module.css";

export function RepositoriesSection() {
  const { data: repositories, isLoading, error } = useRepositories();
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const [createOpen, setCreateOpen] = useState(false);
  const [editTarget, setEditTarget] = useState<RepositoryRecord | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<RepositoryRecord | null>(null);

  const deleteMutation = useMutation({
    mutationFn: (repoName: string) => apiClient.deleteRepository(repoName),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["repositories"] });
      addToast("Repository deleted", "success");
      setDeleteTarget(null);
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to delete repository",
        "error",
      );
    },
  });

  return (
    <>
      {isLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}

      {error && (
        <p className={styles.error}>Failed to load repositories: {(error as Error).message}</p>
      )}

      <Panel
        header={
          <div className={styles.panelHeaderRow}>
            <span className={styles.sectionTitle}>Repositories</span>
            <Button variant="primary" size="sm" onClick={() => setCreateOpen(true)}>
              Add Repository
            </Button>
          </div>
        }
      >
        {repositories && repositories.length === 0 && (
          <p className={styles.empty}>No repositories configured.</p>
        )}
        {repositories && repositories.length > 0 && (
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
        <DeleteConfirmModal
          open={!!deleteTarget}
          onClose={() => setDeleteTarget(null)}
          entityName={deleteTarget.name}
          entityLabel="Repository"
          onConfirm={() => deleteMutation.mutate(deleteTarget.name)}
          isPending={deleteMutation.isPending}
        />
      )}
    </>
  );
}
