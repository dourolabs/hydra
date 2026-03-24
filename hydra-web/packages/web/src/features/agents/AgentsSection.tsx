import { useState } from "react";
import { Panel, Button } from "@hydra/ui";
import type { AgentRecord } from "@hydra/api";
import { useAgents } from "../../hooks/useAgents";
import { LoadingState } from "../../components/LoadingState/LoadingState";
import { ErrorState } from "../../components/ErrorState/ErrorState";
import { EmptyState } from "../../components/EmptyState/EmptyState";
import { AgentRow } from "./AgentRow";
import { AgentCreateModal } from "./AgentCreateModal";
import { AgentEditModal } from "./AgentEditModal";
import { AgentDeleteModal } from "./AgentDeleteModal";
import styles from "./AgentsSection.module.css";

export function AgentsSection() {
  const { data: agents, isLoading: agentsLoading, error: agentsError, refetch } = useAgents();
  const [agentCreateOpen, setAgentCreateOpen] = useState(false);
  const [agentEditTarget, setAgentEditTarget] = useState<AgentRecord | null>(null);
  const [agentDeleteTarget, setAgentDeleteTarget] = useState<AgentRecord | null>(null);

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
          <div className={styles.panelHeaderRow}>
            <span className={styles.sectionTitle}>Agents</span>
            <Button variant="primary" size="sm" onClick={() => setAgentCreateOpen(true)}>
              Add Agent
            </Button>
          </div>
        }
      >
        {agents && agents.length === 0 && <EmptyState message="No agents configured." />}
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
        <AgentDeleteModal
          open={!!agentDeleteTarget}
          agent={agentDeleteTarget}
          onClose={() => setAgentDeleteTarget(null)}
        />
      )}
    </>
  );
}
