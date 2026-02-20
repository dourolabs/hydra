import { useState, useCallback, useMemo } from "react";
import { Link } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Badge, Button, MarkdownViewer, Panel, Select, Spinner, Textarea } from "@metis/ui";
import type { SelectOption } from "@metis/ui";
import type { IssueVersionRecord } from "@metis/api";
import { apiClient } from "../api/client";
import { useIssues } from "../features/issues/useIssues";
import { IssueCreator } from "../features/issues/IssueCreator";
import { useMyAssignedIssues } from "../features/issues/useMyAssignedIssues";
import { useInProgressIssues } from "../features/issues/useInProgressIssues";
import { useRunningJobs } from "../features/jobs/useRunningJobs";
import { useDocument } from "../features/documents/useDocument";
import { useToast } from "../features/toast/useToast";
import { issueToBadgeStatus } from "../utils/statusMapping";
import { descriptionSnippet } from "../utils/text";
import { getRuntime, formatDuration } from "../utils/time";
import { SlideOver } from "../components/SlideOver";
import styles from "./DashboardPage.module.css";

const ISSUE_TYPE_LABELS: Record<string, string> = {
  task: "Task",
  bug: "Bug",
  feature: "Feature",
  chore: "Chore",
  "merge-request": "Merge Request",
  "review-request": "Review",
  unknown: "Issue",
};

const STATUS_OPTIONS: SelectOption[] = [
  { value: "closed", label: "Closed" },
  { value: "failed", label: "Failed" },
  { value: "rejected", label: "Rejected" },
];

/** Extract a document path reference from an issue description (e.g. /designs/foo.md). */
function extractDocumentPath(description: string): string | null {
  const match = description.match(/\/[\w/.-]+\.md\b/);
  return match ? match[0] : null;
}

function timeSince(timestamp: string): string {
  const ms = Date.now() - new Date(timestamp).getTime();
  if (ms < 60_000) return "just now";
  return formatDuration(ms) + " ago";
}

function extractAssignees(issues: IssueVersionRecord[] | undefined): string[] {
  if (!issues) return [];
  const set = new Set<string>();
  for (const record of issues) {
    if (record.issue.assignee) set.add(record.issue.assignee);
  }
  return Array.from(set).sort();
}

/* ---------- Quick Review SlideOver ---------- */

interface QuickReviewPanelProps {
  record: IssueVersionRecord;
  onClose: () => void;
}

function QuickReviewPanel({ record, onClose }: QuickReviewPanelProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const [status, setStatus] = useState("closed");
  const [progress, setProgress] = useState("");

  const docPath = extractDocumentPath(record.issue.description);
  const { data: docRecord } = useDocument(docPath ?? "");

  const mutation = useMutation({
    mutationFn: () =>
      apiClient.updateIssue(record.issue_id, {
        issue: {
          ...record.issue,
          status: status as IssueVersionRecord["issue"]["status"],
          progress: progress || record.issue.progress,
        },
        job_id: null,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["issues"] });
      addToast(`Issue ${record.issue_id} updated`, "success");
      onClose();
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to update issue",
        "error",
      );
    },
  });

  return (
    <div className={styles.reviewContent}>
      {/* Issue description */}
      <div className={styles.reviewSection}>
        <h3 className={styles.reviewSectionTitle}>Description</h3>
        <MarkdownViewer content={record.issue.description} />
      </div>

      {/* Document content (if referenced) */}
      {docPath && (
        <div className={styles.reviewSection}>
          <h3 className={styles.reviewSectionTitle}>Document: {docPath}</h3>
          {docRecord ? (
            <MarkdownViewer content={docRecord.document.body_markdown} />
          ) : (
            <p className={styles.reviewEmpty}>Loading document...</p>
          )}
        </div>
      )}

      {/* Action form */}
      <div className={styles.reviewSection}>
        <h3 className={styles.reviewSectionTitle}>Response</h3>
        <div className={styles.actionForm}>
          <Select
            label="Status"
            options={STATUS_OPTIONS}
            value={status}
            onChange={(e) => setStatus(e.target.value)}
          />
          <Textarea
            placeholder="Provide feedback or progress update..."
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
            {mutation.isPending ? "Submitting..." : "Submit"}
          </Button>
        </div>
      </div>
    </div>
  );
}

/* ---------- Dashboard Page ---------- */

export function DashboardPage() {
  const { data: allIssues } = useIssues();
  const { data: assignedIssues, isLoading: assignedLoading } = useMyAssignedIssues();
  const { data: inProgressIssues, isLoading: inProgressLoading } = useInProgressIssues();
  const { data: runningJobs, isLoading: jobsLoading } = useRunningJobs();

  const [reviewIssue, setReviewIssue] = useState<IssueVersionRecord | null>(null);

  const assignees = useMemo(() => extractAssignees(allIssues), [allIssues]);

  const handleCloseReview = useCallback(() => {
    setReviewIssue(null);
  }, []);

  return (
    <div className={styles.page}>
      {/* Quick Create */}
      <IssueCreator assignees={assignees} />

      {/* Action Required */}
      <Panel
        header={
          <span className={styles.sectionHeader}>
            Action Required
            {assignedIssues.length > 0 && (
              <span className={styles.count}>{assignedIssues.length}</span>
            )}
          </span>
        }
      >
        {assignedLoading && (
          <div className={styles.center}>
            <Spinner size="sm" />
          </div>
        )}
        {!assignedLoading && assignedIssues.length === 0 && (
          <p className={styles.empty}>No items requiring your action.</p>
        )}
        {assignedIssues.length > 0 && (
          <div className={styles.cardList}>
            {assignedIssues.map((record) => (
              <div key={record.issue_id} className={styles.card}>
                <div className={styles.cardTop}>
                  <span className={styles.cardType}>
                    {ISSUE_TYPE_LABELS[record.issue.type] || record.issue.type}
                  </span>
                  <span className={styles.cardDesc}>
                    {descriptionSnippet(record.issue.description, 120)}
                  </span>
                </div>
                <div className={styles.cardBottom}>
                  <span className={styles.cardId}>{record.issue_id}</span>
                  <span className={styles.cardTime}>
                    {timeSince(record.timestamp)}
                  </span>
                  <span className={styles.cardActions}>
                    <Link
                      to={`/issues/${record.issue_id}`}
                      className={styles.cardLink}
                    >
                      Open
                    </Link>
                    <button
                      type="button"
                      className={styles.quickReviewBtn}
                      onClick={() => setReviewIssue(record)}
                    >
                      Quick Review
                    </button>
                  </span>
                </div>
              </div>
            ))}
          </div>
        )}
      </Panel>

      {/* Running Jobs */}
      <Panel
        header={
          <span className={styles.sectionHeader}>
            Running Jobs
            {runningJobs && runningJobs.length > 0 && (
              <span className={styles.count}>{runningJobs.length}</span>
            )}
          </span>
        }
      >
        {jobsLoading && (
          <div className={styles.center}>
            <Spinner size="sm" />
          </div>
        )}
        {!jobsLoading && (!runningJobs || runningJobs.length === 0) && (
          <p className={styles.empty}>No running jobs.</p>
        )}
        {runningJobs && runningJobs.length > 0 && (
          <ul className={styles.jobList}>
            {runningJobs.map((job) => (
              <li key={job.job_id} className={styles.jobRow}>
                <span className={styles.runningDot} />
                <span className={styles.jobId}>{job.job_id}</span>
                {job.task.spawned_from && (
                  <Link
                    to={`/issues/${job.task.spawned_from}`}
                    className={styles.jobIssueLink}
                  >
                    {job.task.spawned_from}
                  </Link>
                )}
                <span className={styles.jobPrompt}>
                  {descriptionSnippet(job.task.prompt, 60)}
                </span>
                <span className={styles.jobRuntime}>
                  {getRuntime(job.task.start_time, job.task.end_time)}
                </span>
                {job.task.spawned_from && (
                  <Link
                    to={`/issues/${job.task.spawned_from}/jobs/${job.job_id}/logs`}
                    className={styles.jobLogLink}
                  >
                    Logs
                  </Link>
                )}
              </li>
            ))}
          </ul>
        )}
      </Panel>

      {/* In Progress Issues */}
      <Panel
        header={
          <span className={styles.sectionHeader}>
            In Progress Issues
            {inProgressIssues.length > 0 && (
              <span className={styles.count}>{inProgressIssues.length}</span>
            )}
          </span>
        }
      >
        {inProgressLoading && (
          <div className={styles.center}>
            <Spinner size="sm" />
          </div>
        )}
        {!inProgressLoading && inProgressIssues.length === 0 && (
          <p className={styles.empty}>No in-progress issues.</p>
        )}
        {inProgressIssues.length > 0 && (
          <ul className={styles.issueList}>
            {inProgressIssues.map((record) => (
              <li key={record.issue_id} className={styles.issueRow}>
                <Badge status={issueToBadgeStatus(record.issue.status)} />
                <Link
                  to={`/issues/${record.issue_id}`}
                  className={styles.issueId}
                >
                  {record.issue_id}
                </Link>
                <span className={styles.issueDesc}>
                  {descriptionSnippet(record.issue.description)}
                </span>
                <span className={styles.issueTime}>
                  {timeSince(record.timestamp)}
                </span>
              </li>
            ))}
          </ul>
        )}
      </Panel>

      {/* Quick Review Slide-Over */}
      <SlideOver
        open={reviewIssue !== null}
        onClose={handleCloseReview}
        title={reviewIssue ? `Review: ${reviewIssue.issue_id}` : ""}
      >
        {reviewIssue && (
          <QuickReviewPanel record={reviewIssue} onClose={handleCloseReview} />
        )}
      </SlideOver>
    </div>
  );
}
