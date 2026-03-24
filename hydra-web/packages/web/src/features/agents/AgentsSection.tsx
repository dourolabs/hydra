import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Panel, Button } from "@hydra/ui";
import type { AgentRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useAgents } from "../../hooks/useAgents";
import { LoadingState } from "../../components/LoadingState/LoadingState";
import { ErrorState } from "../../components/ErrorState/ErrorState";
import { EmptyState } from "../../components/EmptyState/EmptyState";
import { useToast } from "../toast/useToast";
import { ExpandableRow } from "../../components/ExpandableRow/ExpandableRow";
import { AgentCreateModal } from "./AgentCreateModal";
import { AgentEditModal } from "./AgentEditModal";
import { DeleteConfirmModal } from "../../components/DeleteConfirmModal/DeleteConfirmModal";
import sharedStyles from "../../components/SettingsSection/SettingsSection.module.css";
import styles from "./AgentsSection.module.css";

export function AgentsSection() {
  const { data: agents, isLoading: agentsLoading, error: agentsError, refetch } = useAgents();
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
      {agentsLoading && <LoadingState />}

      {agentsError && (
        <ErrorState
          message={`Failed to load agents: ${(agentsError as Error).message}`}
          onRetry={() => refetch()}
        />
      )}

      <Panel
        header={
          <div className={sharedStyles.panelHeaderRow}>
            <span className={sharedStyles.sectionTitle}>Agents</span>
            <Button variant="primary" size="sm" onClick={() => setAgentCreateOpen(true)}>
              Add Agent
            </Button>
          </div>
        }
      >
        {agents && agents.length === 0 && <EmptyState message="No agents configured." />}
        {agents && agents.length > 0 && (
          <div className={sharedStyles.itemList}>
            {agents.map((agent) => (
              <ExpandableRow
                key={agent.name}
                name={agent.name}
                onEdit={() => setAgentEditTarget(agent)}
                onDelete={() => setAgentDeleteTarget(agent)}
                headerExtra={
                  agent.is_assignment_agent ? (
                    <span className={styles.assignmentBadge}>assignment</span>
                  ) : undefined
                }
              >
                <div className={sharedStyles.detailRow}>
                  <span className={sharedStyles.detailLabel}>Prompt Path</span>
                  <span className={sharedStyles.detailValueMono}>
                    {agent.prompt_path || <span className={sharedStyles.dimText}>—</span>}
                  </span>
                </div>
                <div className={sharedStyles.detailRow}>
                  <span className={sharedStyles.detailLabel}>MCP Config Path</span>
                  <span className={sharedStyles.detailValueMono}>
                    {agent.mcp_config_path || <span className={sharedStyles.dimText}>—</span>}
                  </span>
                </div>
                <div className={sharedStyles.detailRow}>
                  <span className={sharedStyles.detailLabel}>Max Tries</span>
                  <span className={sharedStyles.detailValue}>{agent.max_tries}</span>
                </div>
                <div className={sharedStyles.detailRow}>
                  <span className={sharedStyles.detailLabel}>Max Simultaneous</span>
                  <span className={sharedStyles.detailValue}>
                    {agent.max_simultaneous}
                  </span>
                </div>
                <div className={sharedStyles.detailRow}>
                  <span className={sharedStyles.detailLabel}>Assignment Agent</span>
                  <span className={sharedStyles.detailValue}>
                    {agent.is_assignment_agent ? "Yes" : "No"}
                  </span>
                </div>
                <div className={sharedStyles.detailRow}>
                  <span className={sharedStyles.detailLabel}>Secrets</span>
                  <span className={sharedStyles.detailValue}>
                    {agent.secrets && agent.secrets.length > 0 ? (
                      agent.secrets.join(", ")
                    ) : (
                      <span className={sharedStyles.dimText}>None</span>
                    )}
                  </span>
                </div>
              </ExpandableRow>
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
