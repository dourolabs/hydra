import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Panel, Button } from "@hydra/ui";
import type { RepositoryRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useRepositories } from "../../hooks/useRepositories";
import { LoadingState } from "../../components/LoadingState/LoadingState";
import { ErrorState } from "../../components/ErrorState/ErrorState";
import { EmptyState } from "../../components/EmptyState/EmptyState";
import { useToast } from "../toast/useToast";
import { ExpandableRow } from "../../components/ExpandableRow/ExpandableRow";
import { RepositoryCreateModal } from "./RepositoryCreateModal";
import { RepositoryEditModal } from "./RepositoryEditModal";
import { DeleteConfirmModal } from "../../components/DeleteConfirmModal/DeleteConfirmModal";
import sharedStyles from "../../components/SettingsSection/SettingsSection.module.css";

export function RepositoriesSection() {
  const { data: repositories, isLoading, error, refetch } = useRepositories();
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
          <EmptyState message="No repositories configured." />
        )}
        {repositories && repositories.length > 0 && (
          <div className={sharedStyles.itemList}>
            {repositories.map((repo) => {
              const pw = repo.repository.patch_workflow;
              const reviewerCount = pw?.review_requests?.length ?? 0;
              const hasMerge = !!pw?.merge_request?.assignee;
              const parts: string[] = [];
              if (reviewerCount > 0) {
                parts.push(`${reviewerCount} reviewer${reviewerCount === 1 ? "" : "s"}`);
              }
              if (hasMerge) {
                parts.push("merge");
              }
              const workflowSummary = parts.length > 0 ? parts.join(", ") : null;

              return (
                <ExpandableRow
                  key={repo.name}
                  name={repo.name}
                  onEdit={() => setEditTarget(repo)}
                  onDelete={() => setDeleteTarget(repo)}
                >
                  <div className={sharedStyles.detailRow}>
                    <span className={sharedStyles.detailLabel}>Remote URL</span>
                    <span className={sharedStyles.detailValueTerminal}>
                      {repo.repository.remote_url}
                    </span>
                  </div>
                  <div className={sharedStyles.detailRow}>
                    <span className={sharedStyles.detailLabel}>Default Branch</span>
                    <span className={sharedStyles.detailValue}>
                      {repo.repository.default_branch ?? (
                        <span className={sharedStyles.dimText}>—</span>
                      )}
                    </span>
                  </div>
                  <div className={sharedStyles.detailRow}>
                    <span className={sharedStyles.detailLabel}>Default Image</span>
                    <span className={sharedStyles.detailValueTerminal}>
                      {repo.repository.default_image ?? (
                        <span className={sharedStyles.dimText}>—</span>
                      )}
                    </span>
                  </div>
                  <div className={sharedStyles.detailRow}>
                    <span className={sharedStyles.detailLabel}>Patch Workflow</span>
                    <span className={sharedStyles.detailValue}>
                      {workflowSummary ?? <span className={sharedStyles.dimText}>—</span>}
                    </span>
                  </div>
                </ExpandableRow>
              );
            })}
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
