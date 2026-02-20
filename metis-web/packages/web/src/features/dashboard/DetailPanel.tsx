import { useState, useMemo } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Avatar, Badge, Button, MarkdownViewer, Select, Spinner, Textarea } from "@metis/ui";
import type { SelectOption } from "@metis/ui";
import type { IssueVersionRecord } from "@metis/api";
import { apiClient } from "../../api/client";
import { issueToBadgeStatus } from "../../utils/statusMapping";
import { useDocumentByPath } from "../documents/useDocumentByPath";
import { useToast } from "../toast/useToast";
import styles from "./DetailPanel.module.css";

/** Regex to detect document paths in issue text. */
const DOC_PATH_RE = /(?:^|\s)(\/(?:designs|repos|playbooks|research)\/\S+\.md)/m;

function extractDocumentPath(text: string): string | null {
  const match = DOC_PATH_RE.exec(text);
  return match ? match[1] : null;
}

const STATUS_OPTIONS: SelectOption[] = [
  { value: "open", label: "Open" },
  { value: "in-progress", label: "In Progress" },
  { value: "closed", label: "Closed" },
  { value: "failed", label: "Failed" },
  { value: "rejected", label: "Rejected" },
];

interface DetailPanelProps {
  record: IssueVersionRecord;
}

export function DetailPanel({ record }: DetailPanelProps) {
  const { issue } = record;
  const { addToast } = useToast();
  const queryClient = useQueryClient();

  const [status, setStatus] = useState(issue.status);
  const [progress, setProgress] = useState(issue.progress);

  // Reset form when selected issue changes
  const issueId = record.issue_id;
  const [prevIssueId, setPrevIssueId] = useState(issueId);
  if (issueId !== prevIssueId) {
    setPrevIssueId(issueId);
    setStatus(issue.status);
    setProgress(issue.progress);
  }

  const docPath = useMemo(
    () => extractDocumentPath(issue.description + "\n" + issue.progress),
    [issue.description, issue.progress],
  );
  const { data: docRecord, isLoading: docLoading } = useDocumentByPath(docPath);

  const mutation = useMutation({
    mutationFn: () =>
      apiClient.updateIssue(issueId, {
        issue: { ...issue, status, progress },
        job_id: null,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["issues"] });
      queryClient.invalidateQueries({ queryKey: ["issue", issueId] });
      addToast("Issue updated", "success");
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to update issue",
        "error",
      );
    },
  });

  return (
    <div className={styles.panel}>
      {/* Header */}
      <div className={styles.header}>
        <span className={styles.issueId}>{issueId}</span>
        <Badge status={issueToBadgeStatus(issue.status)} />
        <span className={styles.type}>{issue.type}</span>
        {issue.assignee && (
          <span className={styles.assignee}>
            <Avatar name={issue.assignee} size="sm" />
            {issue.assignee}
          </span>
        )}
      </div>

      {/* Description */}
      <div className={styles.section}>
        <h3 className={styles.sectionTitle}>Description</h3>
        {issue.description ? (
          <MarkdownViewer content={issue.description} />
        ) : (
          <p className={styles.empty}>No description.</p>
        )}
      </div>

      {/* Document preview */}
      {docPath && (
        <div className={styles.section}>
          <h3 className={styles.sectionTitle}>Document Preview</h3>
          <p className={styles.docPath}>{docPath}</p>
          {docLoading && <Spinner size="sm" />}
          {docRecord && (
            <div className={styles.docPreview}>
              <MarkdownViewer content={docRecord.document.body_markdown} />
            </div>
          )}
        </div>
      )}

      {/* Action form */}
      <div className={styles.actionForm}>
        <div className={styles.formDivider} />
        <Select
          label="Status"
          options={STATUS_OPTIONS}
          value={status}
          onChange={(e) => setStatus(e.target.value as typeof status)}
        />
        <Textarea
          placeholder="Progress / feedback..."
          value={progress}
          onChange={(e) => setProgress(e.target.value)}
          rows={4}
        />
        <Button
          variant="primary"
          size="sm"
          onClick={() => mutation.mutate()}
          disabled={mutation.isPending}
        >
          {mutation.isPending ? "Submitting..." : "Submit Response"}
        </Button>
      </div>
    </div>
  );
}

export function DetailPanelEmpty() {
  return (
    <div className={styles.emptyState}>
      <p className={styles.emptyText}>Select an item to view details</p>
    </div>
  );
}
