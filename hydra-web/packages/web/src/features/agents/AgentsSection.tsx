import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Avatar, Button } from "@hydra/ui";
import type { AgentRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useAgents } from "../../hooks/useAgents";
import { LoadingState } from "../../components/LoadingState/LoadingState";
import { ErrorState } from "../../components/ErrorState/ErrorState";
import { EmptyState } from "../../components/EmptyState/EmptyState";
import { useToast } from "../toast/useToast";
import { AgentCreateModal } from "./AgentCreateModal";
import { AgentEditModal } from "./AgentEditModal";
import { DeleteConfirmModal } from "../../components/DeleteConfirmModal/DeleteConfirmModal";
import styles from "./AgentsSection.module.css";

interface AgentsSectionProps {
  createOpen: boolean;
  onCreateOpenChange: (open: boolean) => void;
}

export function AgentsSection({ createOpen, onCreateOpenChange }: AgentsSectionProps) {
  const { data: agents, isLoading, error, refetch } = useAgents();
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const [editTarget, setEditTarget] = useState<AgentRecord | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<AgentRecord | null>(null);

  const deleteMutation = useMutation({
    mutationFn: (agentName: string) => apiClient.deleteAgent(agentName),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["agents"] });
      addToast("Agent deleted", "success");
      setDeleteTarget(null);
    },
    onError: (err) => {
      addToast(err instanceof Error ? err.message : "Failed to delete agent", "error");
    },
  });

  return (
    <>
      {isLoading && <LoadingState />}

      {error && (
        <ErrorState
          message={`Failed to load agents: ${(error as Error).message}`}
          onRetry={() => refetch()}
        />
      )}

      {agents && agents.length === 0 && <EmptyState message="No agents configured." />}

      {agents && agents.length > 0 && (
        <div className={styles.cards} data-testid="agents-list">
          {agents.map((agent) => {
            const secretCount = agent.secrets?.length ?? 0;
            return (
              <div
                key={agent.name}
                className={styles.card}
                data-testid={`agents-list-card-${agent.name}`}
              >
                <div className={styles.cardHead}>
                  <Avatar name={agent.name} kind="agent" size="md" />
                  <span className={styles.cardName}>{agent.name}</span>
                  <span className={styles.cardHeadSpacer} />
                  {agent.is_assignment_agent && (
                    <span className={styles.tagChip}>assignment</span>
                  )}
                  {agent.is_default_conversation_agent && (
                    <span className={styles.tagChip}>default chat</span>
                  )}
                </div>

                {agent.prompt_path && (
                  <div className={styles.path} title={agent.prompt_path}>
                    <span className={styles.pathLabel}>prompt</span> {agent.prompt_path}
                  </div>
                )}

                <div className={styles.metaRow}>
                  <span className={styles.metaChip}>
                    <span className={styles.metaChipKey}>tries</span>
                    {agent.max_tries}
                  </span>
                  <span className={styles.metaChip}>
                    <span className={styles.metaChipKey}>concurrency</span>
                    {agent.max_simultaneous}
                  </span>
                  <span className={styles.metaChip}>
                    <span className={styles.metaChipKey}>secrets</span>
                    {secretCount}
                  </span>
                </div>

                <div className={styles.cardFoot}>
                  <span className={styles.cardFootSpacer} />
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setEditTarget(agent)}
                    aria-label={`Configure ${agent.name}`}
                  >
                    Configure
                  </Button>
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => setDeleteTarget(agent)}
                    aria-label={`Delete ${agent.name}`}
                  >
                    Delete
                  </Button>
                </div>
              </div>
            );
          })}
        </div>
      )}

      <AgentCreateModal
        open={createOpen}
        onClose={() => onCreateOpenChange(false)}
        agents={agents ?? []}
      />

      {editTarget && (
        <AgentEditModal
          open={!!editTarget}
          agent={editTarget}
          onClose={() => setEditTarget(null)}
          agents={agents ?? []}
        />
      )}

      {deleteTarget && (
        <DeleteConfirmModal
          open={!!deleteTarget}
          onClose={() => setDeleteTarget(null)}
          entityName={deleteTarget.name}
          entityLabel="Agent"
          onConfirm={() => deleteMutation.mutate(deleteTarget.name)}
          isPending={deleteMutation.isPending}
        />
      )}
    </>
  );
}
