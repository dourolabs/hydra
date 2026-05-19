import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, Icons } from "@hydra/ui";
import type { RepositoryRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useRepositories } from "../../hooks/useRepositories";
import { LoadingState } from "../../components/LoadingState/LoadingState";
import { ErrorState } from "../../components/ErrorState/ErrorState";
import { EmptyState } from "../../components/EmptyState/EmptyState";
import { useToast } from "../toast/useToast";
import { RepositoryCreateModal } from "./RepositoryCreateModal";
import { RepositoryEditModal } from "./RepositoryEditModal";
import { DeleteConfirmModal } from "../../components/DeleteConfirmModal/DeleteConfirmModal";
import styles from "./RepositoriesSection.module.css";

interface RepositoriesSectionProps {
  createOpen: boolean;
  onCreateOpenChange: (open: boolean) => void;
}

function workflowSummary(repo: RepositoryRecord): string | null {
  const pw = repo.repository.patch_workflow;
  const reviewerCount = pw?.review_requests?.length ?? 0;
  const hasMerge = !!pw?.merge_request?.assignee;
  const parts: string[] = [];
  if (reviewerCount > 0) {
    parts.push(`${reviewerCount} reviewer${reviewerCount === 1 ? "" : "s"}`);
  }
  if (hasMerge) parts.push("merge");
  return parts.length > 0 ? parts.join(", ") : null;
}

export function RepositoriesSection({ createOpen, onCreateOpenChange }: RepositoriesSectionProps) {
  const { data: repositories, isLoading, error, refetch } = useRepositories();
  const { addToast } = useToast();
  const queryClient = useQueryClient();
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
      addToast(err instanceof Error ? err.message : "Failed to delete repository", "error");
    },
  });

  return (
    <>
      {isLoading && <LoadingState />}

      {error && (
        <ErrorState
          message={`Failed to load repositories: ${(error as Error).message}`}
          onRetry={() => refetch()}
        />
      )}

      {repositories && repositories.length === 0 && (
        <EmptyState message="No repositories configured." />
      )}

      {repositories && repositories.length > 0 && (
        <div className={styles.tableWrap}>
          <table className={styles.table} data-testid="repositories-list">
            <thead>
              <tr>
                <th className={styles.colName}>Repository</th>
                <th className={styles.colRemote}>Remote URL</th>
                <th className={styles.colBranch}>Default branch</th>
                <th className={styles.colImage}>Image</th>
                <th className={styles.colWorkflow}>Workflow</th>
                <th className={styles.colActions} aria-label="Actions" />
              </tr>
            </thead>
            <tbody>
              {repositories.map((repo) => {
                const workflow = workflowSummary(repo);
                return (
                  <tr
                    key={repo.name}
                    data-testid={`repositories-list-row-${repo.name}`}
                  >
                    <td className={styles.colName}>
                      <span className={styles.nameCell}>
                        <Icons.IconRepo />
                        <span className={styles.nameText}>{repo.name}</span>
                      </span>
                    </td>
                    <td className={styles.colRemote}>
                      <span className={styles.monoDim} title={repo.repository.remote_url}>
                        {repo.repository.remote_url}
                      </span>
                    </td>
                    <td className={styles.colBranch}>
                      {repo.repository.default_branch ? (
                        <span className={styles.mono}>{repo.repository.default_branch}</span>
                      ) : (
                        <span className={styles.dash}>—</span>
                      )}
                    </td>
                    <td className={styles.colImage}>
                      {repo.repository.default_image ? (
                        <span className={styles.monoDim} title={repo.repository.default_image}>
                          {repo.repository.default_image}
                        </span>
                      ) : (
                        <span className={styles.dash}>—</span>
                      )}
                    </td>
                    <td className={styles.colWorkflow}>
                      {workflow ? (
                        <span className={styles.workflowChip}>{workflow}</span>
                      ) : (
                        <span className={styles.dash}>—</span>
                      )}
                    </td>
                    <td className={styles.colActions}>
                      <div className={styles.rowActions}>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => setEditTarget(repo)}
                          aria-label={`Edit ${repo.name}`}
                        >
                          Edit
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => setDeleteTarget(repo)}
                          aria-label={`Delete ${repo.name}`}
                        >
                          Delete
                        </Button>
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}

      <RepositoryCreateModal open={createOpen} onClose={() => onCreateOpenChange(false)} />

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
