import { useState } from "react";
import { Button } from "@hydra/ui";
import type { RepositoryRecord } from "@hydra/api";
import sharedStyles from "../../components/SettingsSection/SettingsSection.module.css";

interface RepositoryRowProps {
  repo: RepositoryRecord;
  onEdit: () => void;
  onDelete: () => void;
}

export function RepositoryRow({ repo, onEdit, onDelete }: RepositoryRowProps) {
  const [expanded, setExpanded] = useState(false);

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
        <span className={sharedStyles.name}>{repo.name}</span>
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
            <span className={sharedStyles.detailLabel}>Remote URL</span>
            <span className={sharedStyles.detailValueMono}>
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
            <span className={sharedStyles.detailValueMono}>
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
        </div>
      )}
    </div>
  );
}
