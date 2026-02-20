import { useState, useMemo } from "react";
import { Badge, Spinner, Tabs } from "@metis/ui";
import type { IssueVersionRecord, JobVersionRecord } from "@metis/api";
import { useAuth } from "../features/auth/useAuth";
import { actorDisplayName } from "../api/auth";
import { useMyAssignedIssues } from "../features/issues/useMyAssignedIssues";
import { useInProgressIssues } from "../features/issues/useInProgressIssues";
import { useRunningJobs } from "../features/jobs/useRunningJobs";
import { issueToBadgeStatus } from "../utils/statusMapping";
import { descriptionSnippet } from "../utils/text";
import { getRuntime } from "../utils/time";
import { DetailPanel } from "../components/DetailPanel";
import { QuickCreate } from "../components/QuickCreate";
import styles from "./DashboardPage.module.css";

const TABS = [
  { id: "inbox", label: "Inbox" },
  { id: "watching", label: "Watching" },
];

function relativeTime(timestamp: string): string {
  const now = Date.now();
  const then = new Date(timestamp).getTime();
  const diffMs = now - then;
  const diffSec = Math.floor(diffMs / 1000);
  if (diffSec < 60) return "just now";
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m ago`;
  const diffHr = Math.floor(diffMin / 60);
  if (diffHr < 24) return `${diffHr}h ago`;
  const diffDay = Math.floor(diffHr / 24);
  return `${diffDay}d ago`;
}

export function DashboardPage() {
  const { user } = useAuth();
  const currentUsername = user ? actorDisplayName(user.actor) : "";

  const [activeTab, setActiveTab] = useState("inbox");
  const [selectedIssueId, setSelectedIssueId] = useState<string | null>(null);

  const { data: assignedIssues, isLoading: assignedLoading } = useMyAssignedIssues(currentUsername);
  const { data: inProgressIssues, isLoading: inProgressLoading } = useInProgressIssues();
  const { data: runningJobs } = useRunningJobs();

  // Build a map of issue_id -> running jobs
  const jobsByIssue = useMemo(() => {
    const map = new Map<string, JobVersionRecord[]>();
    if (runningJobs) {
      for (const job of runningJobs) {
        const issueId = job.task.spawned_from;
        if (issueId) {
          const existing = map.get(issueId) ?? [];
          existing.push(job);
          map.set(issueId, existing);
        }
      }
    }
    return map;
  }, [runningJobs]);

  // Determine which list to show
  const currentList = activeTab === "inbox" ? assignedIssues : inProgressIssues;
  const isLoading = activeTab === "inbox" ? assignedLoading : inProgressLoading;

  // Find the selected record
  const selectedRecord = useMemo(() => {
    if (!selectedIssueId) return null;
    const found = assignedIssues?.find((r: IssueVersionRecord) => r.issue_id === selectedIssueId)
      ?? inProgressIssues?.find((r: IssueVersionRecord) => r.issue_id === selectedIssueId);
    return found ?? null;
  }, [selectedIssueId, assignedIssues, inProgressIssues]);

  return (
    <div className={styles.page}>
      {/* Left pane */}
      <div className={styles.leftPane}>
        <div className={styles.leftHeader}>
          <Tabs tabs={TABS} activeTab={activeTab} onTabChange={setActiveTab} />
        </div>
        <div className={styles.listContent}>
          {isLoading && (
            <div className={styles.center}>
              <Spinner size="sm" />
            </div>
          )}
          {!isLoading && currentList && currentList.length === 0 && (
            <p className={styles.empty}>
              {activeTab === "inbox"
                ? "No assigned items."
                : "No in-progress issues."}
            </p>
          )}
          {currentList && currentList.map((record: IssueVersionRecord) => {
            const isSelected = record.issue_id === selectedIssueId;
            const jobs = jobsByIssue.get(record.issue_id);
            const hasRunningJob = !!jobs && jobs.length > 0;

            return (
              <button
                key={record.issue_id}
                className={`${styles.listItem} ${isSelected ? styles.selected : ""}`}
                onClick={() => setSelectedIssueId(record.issue_id)}
              >
                <div className={styles.itemHeader}>
                  {activeTab === "watching" && hasRunningJob && (
                    <span className={styles.runningIndicator} title="Job running" />
                  )}
                  {activeTab === "inbox" && (
                    <span className={styles.unreadDot} />
                  )}
                  <span className={styles.itemDesc}>
                    {descriptionSnippet(record.issue.description, 60)}
                  </span>
                </div>
                <div className={styles.itemMeta}>
                  <span className={styles.itemId}>{record.issue_id}</span>
                  <Badge status={issueToBadgeStatus(record.issue.status)} />
                  {activeTab === "watching" && hasRunningJob && jobs && (
                    <span className={styles.runtime}>
                      {getRuntime(jobs[0].task.start_time, null)}
                    </span>
                  )}
                  {activeTab === "inbox" && (
                    <span className={styles.timestamp}>{relativeTime(record.timestamp)}</span>
                  )}
                </div>
              </button>
            );
          })}
        </div>
        <QuickCreate />
      </div>

      {/* Right pane */}
      <div className={styles.rightPane}>
        {selectedRecord ? (
          <DetailPanel record={selectedRecord} />
        ) : (
          <div className={styles.emptyDetail}>
            <p className={styles.emptyDetailText}>Select an item to view details</p>
          </div>
        )}
      </div>
    </div>
  );
}
