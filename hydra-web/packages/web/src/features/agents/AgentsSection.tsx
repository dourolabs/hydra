import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Panel, Spinner, Button } from "@hydra/ui";
import type { AgentRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useAgents } from "../../hooks/useAgents";
import { useToast } from "../toast/useToast";
import { AgentRow } from "./AgentRow";
import { AgentCreateModal } from "./AgentCreateModal";
import { AgentEditModal } from "./AgentEditModal";
import { DeleteConfirmModal } from "../../components/DeleteConfirmModal/DeleteConfirmModal";
import styles from "./AgentsSection.module.css";

export function AgentsSection() {
  const { data: agents, isLoading: agentsLoading, error: agentsError } = useAgents();
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const [agentCreateOpen, setAgentCreateOpen] = useState(false);
  const [agentEditTarget, setAgentEditTarget] = useState<AgentRecord | null>(null);
  const [agentDeleteTarget, setAgentDeleteTarget] = useState<AgentRecord | null>(null);

  const deleteMutation = useMutation({
    mutationFn: (agentName: string) => apiClient.deleteAgent(agentName),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["agents"] });
      addToast("Agent deleted", "success");
      setAgentDeleteTarget(null);
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to delete agent",
        "error",
      );
    },
  });

  return (
    <>
      {agentsLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}

      {agentsError && (
        <p className={styles.error}>Failed to load agents: {(agentsError as Error).message}</p>
      )}

      <Panel
        header={
          <div className={styles.panelHeaderRow}>
            <span className={styles.sectionTitle}>Agents</span>
            <Button variant="primary" size="sm" onClick={() => setAgentCreateOpen(true)}>
              Add Agent
            </Button>
          </div>
        }
      >
        {agents && agents.length === 0 && <p className={styles.empty}>No agents configured.</p>}
        {agents && agents.length > 0 && (
          <div className={styles.agentList}>
            {agents.map((agent) => (
              <AgentRow
                key={agent.name}
                agent={agent}
                onEdit={() => setAgentEditTarget(agent)}
                onDelete={() => setAgentDeleteTarget(agent)}
              />
            ))}
          </div>
        )}
      </Panel>

      <AgentCreateModal
        open={agentCreateOpen}
        onClose={() => setAgentCreateOpen(false)}
        agents={agents ?? []}
      />

      {agentEditTarget && (
        <AgentEditModal
          open={!!agentEditTarget}
          agent={agentEditTarget}
          onClose={() => setAgentEditTarget(null)}
          agents={agents ?? []}
        />
      )}

      {agentDeleteTarget && (
        <DeleteConfirmModal
          open={!!agentDeleteTarget}
          onClose={() => setAgentDeleteTarget(null)}
          entityName={agentDeleteTarget.name}
          entityLabel="Agent"
          onConfirm={() => deleteMutation.mutate(agentDeleteTarget.name)}
          isPending={deleteMutation.isPending}
        />
      )}
    </>
  );
}
