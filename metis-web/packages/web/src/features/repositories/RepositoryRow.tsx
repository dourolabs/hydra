import { useState } from "react";
import { Button } from "@hydra/ui";
import type { RepositoryRecord } from "@hydra/api";
import styles from "./RepositoriesSection.module.css";

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
    <div className={styles.repoItem}>
      <button
        type="button"
        className={styles.repoHeader}
        onClick={() => setExpanded((prev) => !prev)}
        aria-expanded={expanded}
      >
        <span className={styles.chevron} aria-hidden="true">
          {expanded ? "▾" : "▸"}
        </span>
        <span className={styles.repoName}>{repo.name}</span>
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
        <div className={styles.repoDetails}>
          <div className={styles.detailRow}>
            <span className={styles.detailLabel}>Remote URL</span>
            <span className={styles.detailValueMono}>
              {repo.repository.remote_url}
            </span>
          </div>
          <div className={styles.detailRow}>
            <span className={styles.detailLabel}>Default Branch</span>
            <span className={styles.detailValue}>
              {repo.repository.default_branch ?? (
                <span className={styles.dimText}>—</span>
              )}
            </span>
          </div>
          <div className={styles.detailRow}>
            <span className={styles.detailLabel}>Default Image</span>
            <span className={styles.detailValueMono}>
              {repo.repository.default_image ?? (
                <span className={styles.dimText}>—</span>
              )}
            </span>
          </div>
          <div className={styles.detailRow}>
            <span className={styles.detailLabel}>Patch Workflow</span>
            <span className={styles.detailValue}>
              {workflowSummary ?? <span className={styles.dimText}>—</span>}
            </span>
          </div>
        </div>
      )}
    </div>
  );
}
