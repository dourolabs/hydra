import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Button, EmptyState, ErrorState, Icons, LoadingState } from "@hydra/ui";
import type { RepositoryRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useRepositories } from "../../hooks/useRepositories";
import { useMediaQuery } from "../../hooks/useMediaQuery";
import { useToast } from "../toast/useToast";
import { RepositoryRailRow } from "../related/RailRow";
import { RepositoryCreateModal } from "./RepositoryCreateModal";
import { RepositoryEditModal } from "./RepositoryEditModal";
import { MergePolicySummary } from "./MergePolicySummary";
import { DeleteConfirmModal } from "../../components/DeleteConfirmModal/DeleteConfirmModal";
import styles from "./RepositoriesSection.module.css";

const MOBILE_QUERY = "(max-width: 768px)";

interface RepositoriesSectionProps {
  createOpen: boolean;
  onCreateOpenChange: (open: boolean) => void;
}

export function RepositoriesSection({ createOpen, onCreateOpenChange }: RepositoriesSectionProps) {
  const { data: repositories, isLoading, error, refetch } = useRepositories();
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const isMobile = useMediaQuery(MOBILE_QUERY);
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

      {repositories && repositories.length > 0 && isMobile && (
        <div className={styles.mobileList} data-testid="repositories-list">
          {repositories.map((repo) => (
            <div
              key={repo.name}
              className={styles.mobileItem}
              data-testid={`repositories-list-row-${repo.name}`}
            >
              <RepositoryRailRow record={repo} />
              <div className={styles.mobilePolicy}>
                <MergePolicySummary
                  policy={repo.repository.merge_policy}
                  repoName={repo.name}
                />
              </div>
              <div className={styles.mobileActions}>
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
            </div>
          ))}
        </div>
      )}

      {repositories && repositories.length > 0 && !isMobile && (
        <div className={styles.tableWrap}>
          <table className={styles.table} data-testid="repositories-list">
            <thead>
              <tr>
                <th className={styles.colName}>Repository</th>
                <th className={styles.colRemote}>Remote URL</th>
                <th className={styles.colBranch}>Default branch</th>
                <th className={styles.colPolicy}>Merge policy</th>
                <th className={styles.colActions} aria-label="Actions" />
              </tr>
            </thead>
            <tbody>
              {repositories.map((repo) => {
                return (
                  <tr key={repo.name} data-testid={`repositories-list-row-${repo.name}`}>
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
                    <td className={styles.colPolicy}>
                      <MergePolicySummary
                        policy={repo.repository.merge_policy}
                        repoName={repo.name}
                      />
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
