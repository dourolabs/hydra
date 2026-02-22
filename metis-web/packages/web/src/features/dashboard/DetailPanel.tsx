import { useState, useMemo } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Avatar, Badge, Button, MarkdownViewer, Select, Spinner, Textarea } from "@metis/ui";
import type { SelectOption } from "@metis/ui";
import type { PatchVersionRecord } from "@metis/api";
import { apiClient } from "../../api/client";
import { issueToBadgeStatus, patchToBadgeStatus } from "../../utils/statusMapping";
import { useDocumentByPath } from "../documents/useDocumentByPath";
import { useIssue } from "../issues/useIssue";
import { usePatchesByIssue } from "../patches/usePatchesByIssue";
import { useToast } from "../toast/useToast";
import { DiffViewer } from "./DiffViewer";
import styles from "./DetailPanel.module.css";

/** Regex to detect document paths in issue text. */
const DOC_PATH_RE = /(?:^|\s)(\/\S+\.md)/m;

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
  { value: "dropped", label: "Dropped" },
];

interface DetailPanelProps {
  issueId: string;
}

export function DetailPanel({ issueId }: DetailPanelProps) {
  const { data: record, isLoading: recordLoading } = useIssue(issueId);
  const issue = record?.issue;
  const { addToast } = useToast();
  const queryClient = useQueryClient();

  const [status, setStatus] = useState(issue?.status ?? "open");
  const [progress, setProgress] = useState(issue?.progress ?? "");

  // Reset form when selected issue or its data changes
  const [prevIssueId, setPrevIssueId] = useState(issueId);
  const [prevVersion, setPrevVersion] = useState(record?.version);
  if (issueId !== prevIssueId || (record && record.version !== prevVersion)) {
    setPrevIssueId(issueId);
    setPrevVersion(record?.version);
    if (issue) {
      setStatus(issue.status);
      setProgress(issue.progress);
    }
  }

  const docPath = useMemo(
    () => issue ? extractDocumentPath(issue.description + "\n" + issue.progress) : null,
    [issue],
  );
  const { data: docRecord, isLoading: docLoading } = useDocumentByPath(docPath);

  const [searchParams] = useSearchParams();
  const activeTab = searchParams.get("tab") ?? "inbox";

  const patchIds = issue?.patches ?? [];
  const { data: patches, isLoading: patchesLoading } = usePatchesByIssue(patchIds);

  const mutation = useMutation({
    mutationFn: () => {
      if (!issue) throw new Error("Issue not loaded");
      return apiClient.updateIssue(issueId, {
        issue: { ...issue, status, progress },
        job_id: null,
      });
    },
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

  if (recordLoading || !issue) {
    return (
      <div className={styles.panel}>
        <div className={styles.header}>
          <span className={styles.issueIdLink}>{issueId}</span>
        </div>
        <div className={styles.section}>
          <Spinner size="sm" />
        </div>
      </div>
    );
  }

  return (
    <div className={styles.panel}>
      {/* Header */}
      <div className={styles.header}>
        <Link to={`/issues/${issueId}?from=dashboard&tab=${activeTab}`} className={styles.issueIdLink}>{issueId}</Link>
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
          {docRecord ? (
            <Link to={`/documents/${docRecord.document_id}?from=dashboard&issueId=${issueId}`} className={styles.docPathLink}>{docPath}</Link>
          ) : (
            <p className={styles.docPath}>{docPath}</p>
          )}
          {docLoading && <Spinner size="sm" />}
          {docRecord && (
            <div className={styles.docPreview}>
              <MarkdownViewer content={docRecord.document.body_markdown} />
            </div>
          )}
        </div>
      )}

      {/* Patch previews */}
      {patchIds.length > 0 && (
        <div className={styles.section}>
          <h3 className={styles.sectionTitle}>Patches</h3>
          {patchesLoading && <Spinner size="sm" />}
          {patches.map((patchRecord) => (
            <PatchPreview key={patchRecord.patch_id} record={patchRecord} issueId={issueId} />
          ))}
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

function PatchPreview({ record, issueId }: { record: PatchVersionRecord; issueId: string }) {
  const { patch } = record;

  return (
    <div className={styles.patchCard}>
      <div className={styles.patchHeader}>
        <Link to={`/patches/${record.patch_id}?from=dashboard&issueId=${issueId}`} className={styles.patchIdLink}>{record.patch_id}</Link>
        <Badge status={patchToBadgeStatus(patch.status)} />
      </div>
      <p className={styles.patchTitle}>{patch.title}</p>

      {patch.github?.url && (
        <a
          href={patch.github.url}
          target="_blank"
          rel="noopener noreferrer"
          className={styles.ghLink}
        >
          {patch.github.owner}/{patch.github.repo}#{String(patch.github.number)} ↗
        </a>
      )}

      {patch.reviews.length > 0 && (
        <div className={styles.patchReviews}>
          {patch.reviews.map((review, i) => (
            <span key={i} className={styles.patchReviewChip}>
              <Avatar name={review.author} size="sm" />
              {review.author}
              {" \u2014 "}
              {review.is_approved ? "approved" : "changes requested"}
            </span>
          ))}
        </div>
      )}

      {patch.diff && <DiffViewer diff={patch.diff} />}
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
