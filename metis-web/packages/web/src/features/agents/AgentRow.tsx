import { useState } from "react";
import { Button } from "@metis/ui";
import type { AgentRecord } from "@metis/api";
import styles from "./AgentsSection.module.css";

interface AgentRowProps {
  agent: AgentRecord;
  onEdit: () => void;
  onDelete: () => void;
}

export function AgentRow({ agent, onEdit, onDelete }: AgentRowProps) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className={styles.agentItem}>
      <button
        type="button"
        className={styles.agentHeader}
        onClick={() => setExpanded((prev) => !prev)}
        aria-expanded={expanded}
      >
        <span className={styles.chevron} aria-hidden="true">
          {expanded ? "▾" : "▸"}
        </span>
        <span className={styles.agentName}>{agent.name}</span>
        {agent.is_assignment_agent && (
          <span className={styles.assignmentBadge}>assignment</span>
        )}
        <div className={styles.rowActions}>
          <Button
            variant="ghost"
            size="sm"
            onClick={(e) => {
              e.stopPropagation();
              onEdit();
            }}
          >
            Edit
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={(e) => {
              e.stopPropagation();
              onDelete();
            }}
          >
            Delete
          </Button>
        </div>
      </button>
      {expanded && (
        <div className={styles.agentDetails}>
          <div className={styles.detailRow}>
            <span className={styles.detailLabel}>Prompt Path</span>
            <span className={styles.detailValueMono}>
              {agent.prompt_path || <span className={styles.dimText}>—</span>}
            </span>
          </div>
          <div className={styles.detailRow}>
            <span className={styles.detailLabel}>Max Tries</span>
            <span className={styles.detailValue}>{agent.max_tries}</span>
          </div>
          <div className={styles.detailRow}>
            <span className={styles.detailLabel}>Max Simultaneous</span>
            <span className={styles.detailValue}>
              {agent.max_simultaneous}
            </span>
          </div>
          <div className={styles.detailRow}>
            <span className={styles.detailLabel}>Assignment Agent</span>
            <span className={styles.detailValue}>
              {agent.is_assignment_agent ? "Yes" : "No"}
            </span>
          </div>
        </div>
      )}
    </div>
  );
}
