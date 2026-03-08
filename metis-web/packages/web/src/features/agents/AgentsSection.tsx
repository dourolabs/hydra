import { useState } from "react";
import { Panel, Spinner, Button } from "@metis/ui";
import type { AgentRecord } from "@metis/api";
import { useAgents } from "../../hooks/useAgents";
import { AgentRow } from "./AgentRow";
import { AgentCreateModal } from "./AgentCreateModal";
import { AgentEditModal } from "./AgentEditModal";
import { AgentDeleteModal } from "./AgentDeleteModal";
import styles from "./AgentsSection.module.css";

export function AgentsSection() {
  const {
    data: agents,
    isLoading: agentsLoading,
    error: agentsError,
  } = useAgents();
  const [agentCreateOpen, setAgentCreateOpen] = useState(false);
  const [agentEditTarget, setAgentEditTarget] = useState<AgentRecord | null>(
    null,
  );
  const [agentDeleteTarget, setAgentDeleteTarget] =
    useState<AgentRecord | null>(null);

  return (
    <>
      <div className={styles.headerRow}>
        <span className={styles.sectionTitle}>Agents</span>
        <Button
          variant="primary"
          size="sm"
          onClick={() => setAgentCreateOpen(true)}
        >
          Add Agent
        </Button>
      </div>

      {agentsLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}

      {agentsError && (
        <p className={styles.error}>
          Failed to load agents: {(agentsError as Error).message}
        </p>
      )}

      {agents && agents.length === 0 && (
        <p className={styles.empty}>No agents configured.</p>
      )}

      {agents && agents.length > 0 && (
        <Panel>
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
        </Panel>
      )}

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
