import { useState } from "react";
import { Button } from "@hydra/ui";
import type { AgentRecord } from "@hydra/api";
import sharedStyles from "../../components/SettingsSection/SettingsSection.module.css";
import styles from "./AgentsSection.module.css";

interface AgentRowProps {
  agent: AgentRecord;
  onEdit: () => void;
  onDelete: () => void;
}

export function AgentRow({ agent, onEdit, onDelete }: AgentRowProps) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className={sharedStyles.item}>
      <button
        type="button"
        className={sharedStyles.header}
        onClick={() => setExpanded((prev) => !prev)}
        aria-expanded={expanded}
      >
        <span className={sharedStyles.chevron} aria-hidden="true">
          {expanded ? "▾" : "▸"}
        </span>
        <span className={sharedStyles.name}>{agent.name}</span>
        {agent.is_assignment_agent && (
          <span className={styles.assignmentBadge}>assignment</span>
        )}
        <div className={sharedStyles.rowActions}>
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
        <div className={sharedStyles.details}>
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
        </div>
      )}
    </div>
  );
}
