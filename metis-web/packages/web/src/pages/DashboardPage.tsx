import { useState, useMemo, useCallback } from "react";
import { Link } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Panel, Spinner, Badge, Select, Textarea, Button, MarkdownViewer } from "@metis/ui";
import type { SelectOption } from "@metis/ui";
import type { IssueVersionRecord, IssueStatus, JobVersionRecord } from "@metis/api";
import { useIssues } from "../features/issues/useIssues";
import { IssueCreator } from "../features/issues/IssueCreator";
import { useMyAssignedIssues } from "../features/issues/useMyAssignedIssues";
import { useInProgressIssues } from "../features/issues/useInProgressIssues";
import { useRunningJobs } from "../features/jobs/useRunningJobs";
import { useToast } from "../features/toast/useToast";
import { apiClient } from "../api/client";
import { issueToBadgeStatus } from "../utils/statusMapping";
import { descriptionSnippet } from "../utils/text";
import { getRuntime } from "../utils/time";
import styles from "./DashboardPage.module.css";

const STATUS_OPTIONS: SelectOption[] = [
  { value: "open", label: "Open" },
  { value: "in-progress", label: "In Progress" },
  { value: "closed", label: "Closed" },
  { value: "failed", label: "Failed" },
  { value: "blocked", label: "Blocked" },
  { value: "rejected", label: "Rejected" },
];

function extractAssignees(issues: IssueVersionRecord[] | undefined): string[] {
  if (!issues) return [];
  const set = new Set<string>();
  for (const record of issues) {
    if (record.issue.assignee) set.add(record.issue.assignee);
  }
  return Array.from(set).sort();
}

function formatRelativeTime(timestamp: string): string {
  const diff = Date.now() - new Date(timestamp).getTime();
  const seconds = Math.floor(diff / 1000);
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

// --- My Queue Item ---

interface QueueItemProps {
  record: IssueVersionRecord;
  expanded: boolean;
  onToggle: () => void;
}

function QueueItem({ record, expanded, onToggle }: QueueItemProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const [status, setStatus] = useState<IssueStatus>(record.issue.status);
  const [progress, setProgress] = useState(record.issue.progress ?? "");

  const mutation = useMutation({
    mutationFn: (params: { status: string; progress: string }) =>
      apiClient.updateIssue(record.issue_id, {
        issue: {
          ...record.issue,
          status: params.status as IssueVersionRecord["issue"]["status"],
          progress: params.progress,
        },
        job_id: null,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["issues"] });
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
    mutation.mutate({ status, progress });
  };

  return (
    <div className={styles.queueItem}>
      <div
        className={styles.queueItemHeader}
        onClick={onToggle}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") onToggle();
        }}
      >
        <Badge status={issueToBadgeStatus(record.issue.status)} />
        <span className={styles.issueId}>{record.issue_id}</span>
        <span className={styles.issueType}>{record.issue.type}</span>
        <span className={styles.issueDesc}>
          {descriptionSnippet(record.issue.description)}
        </span>
        <span className={styles.expandIcon}>{expanded ? "\u25BC" : "\u25B6"}</span>
      </div>
      {expanded && (
        <div className={styles.queueItemExpanded}>
          <div className={styles.descriptionContent}>
            <MarkdownViewer content={record.issue.description} />
          </div>
          <div className={styles.actionForm}>
            <div className={styles.actionRow}>
              <Select
                label="Status"
                options={STATUS_OPTIONS}
                value={status}
                onChange={(e) => setStatus(e.target.value as IssueStatus)}
              />
              <Button
                variant="primary"
                size="sm"
                onClick={handleSubmit}
                disabled={mutation.isPending}
              >
                {mutation.isPending ? "Updating..." : "Submit"}
              </Button>
            </div>
            <Textarea
              label="Progress"
              value={progress}
              onChange={(e) => setProgress(e.target.value)}
              rows={3}
              placeholder="Add progress notes..."
            />
          </div>
        </div>
      )}
    </div>
  );
}

// --- In Progress Item ---

interface ProgressItemProps {
  record: IssueVersionRecord;
  jobs: JobVersionRecord[];
}

function ProgressItem({ record, jobs }: ProgressItemProps) {
  const issueJobs = jobs.filter((j) => j.task?.spawned_from === record.issue_id);

  return (
    <div className={styles.progressItem}>
      <div className={styles.progressItemHeader}>
        <Badge status={issueToBadgeStatus(record.issue.status)} />
        <Link to={`/issues/${record.issue_id}`} className={styles.issueId}>
          {record.issue_id}
        </Link>
        <span className={styles.progressDesc}>
          {descriptionSnippet(record.issue.description, 60)}
        </span>
      </div>
      {issueJobs.length > 0 ? (
        issueJobs.map((job) => (
          <div key={job.job_id} className={styles.jobRow}>
            <span className={styles.spinner} />
            <span className={styles.jobId}>{job.job_id}</span>
            <span className={styles.runtime}>
              {getRuntime(job.task?.start_time, null)}
            </span>
            <Link
              to={`/issues/${record.issue_id}/jobs/${job.job_id}/logs`}
              className={styles.logsLink}
            >
              View Logs
            </Link>
          </div>
        ))
      ) : (
        <span className={styles.noJobs}>No active jobs</span>
      )}
    </div>
  );
}

// --- Recent Activity ---

interface ActivityEntry {
  id: string;
  timestamp: string;
  text: string;
}

function buildRecentActivity(
  issues: IssueVersionRecord[] | undefined,
  jobs: JobVersionRecord[] | undefined,
): ActivityEntry[] {
  const entries: ActivityEntry[] = [];

  if (issues) {
    for (const issue of issues) {
      entries.push({
        id: `issue-${issue.issue_id}-${issue.version}`,
        timestamp: issue.timestamp,
        text: `${issue.issue_id} status: ${issue.issue.status}`,
      });
    }
  }

  if (jobs) {
    for (const job of jobs) {
      entries.push({
        id: `job-${job.job_id}-${job.version}`,
        timestamp: job.timestamp,
        text: `Job ${job.job_id} ${job.task?.status ?? "unknown"}`,
      });
    }
  }

  entries.sort((a, b) => b.timestamp.localeCompare(a.timestamp));
  return entries.slice(0, 15);
}

// --- Dashboard Page ---

export function DashboardPage() {
  const { data: allIssues } = useIssues();
  const { data: assignedIssues, isLoading: assignedLoading } = useMyAssignedIssues();
  const { data: inProgressIssues, isLoading: progressLoading } = useInProgressIssues();
  const { data: runningJobs } = useRunningJobs();

  const [expandedId, setExpandedId] = useState<string | null>(null);

  const assignees = useMemo(() => extractAssignees(allIssues), [allIssues]);

  const recentActivity = useMemo(
    () => buildRecentActivity(allIssues, runningJobs),
    [allIssues, runningJobs],
  );

  const toggleExpand = useCallback((issueId: string) => {
    setExpandedId((prev) => (prev === issueId ? null : issueId));
  }, []);

  return (
    <div className={styles.page}>
      <IssueCreator assignees={assignees} />

      <div className={styles.columns}>
        {/* My Queue Panel */}
        <Panel
          header={
            <span className={styles.panelTitle}>
              My Queue
              {assignedIssues && (
                <span className={styles.count}>({assignedIssues.length})</span>
              )}
            </span>
          }
        >
          {assignedLoading && (
            <div className={styles.center}>
              <Spinner size="sm" />
            </div>
          )}
          {assignedIssues && assignedIssues.length === 0 && (
            <p className={styles.empty}>No items in your queue.</p>
          )}
          {assignedIssues &&
            assignedIssues.map((record) => (
              <QueueItem
                key={record.issue_id}
                record={record}
                expanded={expandedId === record.issue_id}
                onToggle={() => toggleExpand(record.issue_id)}
              />
            ))}
        </Panel>

        {/* In Progress Panel */}
        <Panel
          header={
            <span className={styles.panelTitle}>
              In Progress
              {inProgressIssues && (
                <span className={styles.count}>({inProgressIssues.length})</span>
              )}
            </span>
          }
        >
          {progressLoading && (
            <div className={styles.center}>
              <Spinner size="sm" />
            </div>
          )}
          {inProgressIssues && inProgressIssues.length === 0 && (
            <p className={styles.empty}>No issues currently in progress.</p>
          )}
          {inProgressIssues &&
            inProgressIssues.map((record) => (
              <ProgressItem
                key={record.issue_id}
                record={record}
                jobs={runningJobs ?? []}
              />
            ))}
        </Panel>
      </div>

      {/* Recent Activity */}
      <Panel
        header={
          <span className={styles.panelTitle}>Recent Activity</span>
        }
      >
        {recentActivity.length === 0 && (
          <p className={styles.empty}>No recent activity.</p>
        )}
        {recentActivity.map((entry) => (
          <div key={entry.id} className={styles.activityItem}>
            <span className={styles.activityTime}>
              {formatRelativeTime(entry.timestamp)}
            </span>
            <span className={styles.activityText}>{entry.text}</span>
          </div>
        ))}
      </Panel>
    </div>
  );
}
