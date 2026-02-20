import { useState, useMemo } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Avatar, Badge, Button, MarkdownViewer, Select, Spinner, Textarea } from "@metis/ui";
import type { SelectOption, BadgeStatus } from "@metis/ui";
import type { IssueVersionRecord } from "@metis/api";
import { apiClient } from "../api/client";
import { issueToBadgeStatus } from "../utils/statusMapping";
import { formatTimestamp } from "../utils/time";
import { useDocumentByPath } from "../features/documents/useDocumentByPath";
import { useToast } from "../features/toast/useToast";
import styles from "./DetailPanel.module.css";

interface DetailPanelProps {
  record: IssueVersionRecord;
}

const STATUS_OPTIONS: SelectOption[] = [
  { value: "", label: "No change" },
  { value: "open", label: "Open" },
  { value: "in-progress", label: "In Progress" },
  { value: "closed", label: "Closed" },
  { value: "failed", label: "Failed" },
  { value: "dropped", label: "Dropped" },
  { value: "rejected", label: "Rejected" },
];

/** Extract a document path reference from the issue description (e.g., /designs/foo.md). */
function extractDocumentPath(description: string): string | null {
  const match = description.match(/\/[\w/-]+\.md\b/);
  return match ? match[0] : null;
}

export function DetailPanel({ record }: DetailPanelProps) {
  const { issue } = record;
  const { addToast } = useToast();
  const queryClient = useQueryClient();

  const [status, setStatus] = useState("");
  const [progress, setProgress] = useState("");

  const docPath = useMemo(
    () => extractDocumentPath(issue.description ?? ""),
    [issue.description],
  );
  const { data: docRecord, isLoading: docLoading } = useDocumentByPath(docPath);

  const mutation = useMutation({
    mutationFn: (params: { status?: string; progress?: string }) => {
      const updatedIssue = {
        ...issue,
        ...(params.status && { status: params.status as typeof issue.status }),
        ...(params.progress !== undefined && { progress: params.progress }),
      };
      return apiClient.updateIssue(record.issue_id, {
        issue: updatedIssue,
        job_id: null,
      });
    },
    onSuccess: () => {
      setStatus("");
      setProgress("");
      queryClient.invalidateQueries({ queryKey: ["issues"] });
      queryClient.invalidateQueries({ queryKey: ["issue", record.issue_id] });
      addToast(`Issue ${record.issue_id} updated`, "success");
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to update issue",
        "error",
      );
    },
  });

  const handleSubmit = () => {
    const updates: { status?: string; progress?: string } = {};
    if (status) updates.status = status;
    if (progress.trim()) updates.progress = progress.trim();
    if (!updates.status && !updates.progress) return;
    mutation.mutate(updates);
  };

  return (
    <div className={styles.panel}>
      {/* Header */}
      <div className={styles.header}>
        <span className={styles.issueId}>{record.issue_id}</span>
        <Badge status={issueToBadgeStatus(issue.status) as BadgeStatus} />
        <span className={styles.type}>{issue.type}</span>
      </div>

      {/* Metadata */}
      <div className={styles.meta}>
        {issue.assignee && (
          <div className={styles.metaItem}>
            <span className={styles.metaLabel}>Assignee</span>
            <span className={styles.metaValue}>
              <Avatar name={issue.assignee} size="sm" />
              {issue.assignee}
            </span>
          </div>
        )}
        {issue.creator && (
          <div className={styles.metaItem}>
            <span className={styles.metaLabel}>Creator</span>
            <span className={styles.metaValue}>
              <Avatar name={issue.creator} size="sm" />
              {issue.creator}
            </span>
          </div>
        )}
        <div className={styles.metaItem}>
          <span className={styles.metaLabel}>Updated</span>
          <span className={styles.metaValue}>
            {formatTimestamp(record.timestamp)}
          </span>
        </div>
      </div>

      {/* Description */}
      <div className={styles.section}>
        <div className={styles.sectionTitle}>Description</div>
        <div className={styles.sectionBody}>
          {issue.description ? (
            <MarkdownViewer content={issue.description} />
          ) : (
            <p className={styles.empty}>No description.</p>
          )}
        </div>
      </div>

      {/* Progress */}
      {issue.progress && (
        <div className={styles.section}>
          <div className={styles.sectionTitle}>Progress</div>
          <div className={styles.sectionBody}>
            <MarkdownViewer content={issue.progress} />
          </div>
        </div>
      )}

      {/* Document Preview */}
      {docPath && (
        <div className={styles.section}>
          <div className={styles.sectionTitle}>Document Preview</div>
          <div className={styles.sectionBody}>
            {docLoading && (
              <div className={styles.docLoading}>
                <Spinner size="sm" />
                <span>Loading {docPath}...</span>
              </div>
            )}
            {docRecord && (
              <div className={styles.docPreview}>
                <div className={styles.docPath}>{docRecord.document.path ?? docPath}</div>
                <MarkdownViewer content={docRecord.document.body_markdown} />
              </div>
            )}
            {!docLoading && !docRecord && (
              <p className={styles.empty}>Document not found: {docPath}</p>
            )}
          </div>
        </div>
      )}

      {/* Action Form */}
      <div className={styles.actionForm}>
        <div className={styles.sectionTitle}>Update Issue</div>
        <Select
          label="Status"
          options={STATUS_OPTIONS}
          value={status}
          onChange={(e) => setStatus(e.target.value)}
        />
        <Textarea
          label="Progress / Feedback"
          placeholder="Add progress notes or feedback..."
          value={progress}
          onChange={(e) => setProgress(e.target.value)}
          rows={3}
        />
        <Button
          variant="primary"
          size="sm"
          onClick={handleSubmit}
          disabled={(!status && !progress.trim()) || mutation.isPending}
        >
          {mutation.isPending ? "Submitting..." : "Submit Response"}
        </Button>
      </div>
    </div>
  );
}
