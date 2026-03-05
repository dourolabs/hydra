import { useState } from "react";
import { Link, useParams } from "react-router-dom";
import { Badge, Spinner, Tabs } from "@metis/ui";
import { jobToBadgeStatus } from "../utils/statusMapping";
import { getRuntime } from "../utils/time";
import { useJob } from "../features/jobs/useJob";
import { JobLogViewer } from "../features/jobs/JobLogViewer";
import { JobSettings } from "../features/jobs/JobSettings";
import { ApiError } from "../api/client";
import { Breadcrumbs } from "../layout/Breadcrumbs";
import styles from "./JobLogPage.module.css";

const TABS = [
  { id: "logs", label: "Logs" },
  { id: "settings", label: "Settings" },
];

export function JobLogPage() {
  const { issueId, jobId } = useParams<{
    issueId: string;
    jobId: string;
  }>();
  const { data: record, isLoading, error } = useJob(jobId ?? "");
  const [activeTab, setActiveTab] = useState("logs");

  return (
    <div className={styles.page}>
      <Breadcrumbs
        items={[
          { label: "Dashboard", to: "/" },
          { label: `Issue ${issueId}`, to: `/issues/${issueId}` },
        ]}
        current={`Job ${jobId}`}
      />

      {isLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}

      {error && (
        <div className={styles.errorContainer}>
          {error instanceof ApiError && error.status === 404 ? (
            <p className={styles.error}>
              Job <strong>{jobId}</strong> not found.
            </p>
          ) : (
            <p className={styles.error}>
              Failed to load job: {(error as Error).message}
            </p>
          )}
        </div>
      )}

      {record && (
        <>
          {/* Job metadata header */}
          <div className={styles.header}>
            <div className={styles.headerTop}>
              <span className={styles.jobId}>{record.job_id}</span>
              <Badge status={jobToBadgeStatus(record.task.status)} />
            </div>
            <div className={styles.meta}>
              <div className={styles.metaItem}>
                <span className={styles.metaLabel}>Issue</span>
                <Link to={`/issues/${issueId}`} className={styles.metaLink}>
                  {issueId}
                </Link>
              </div>
              <div className={styles.metaItem}>
                <span className={styles.metaLabel}>Runtime</span>
                <span className={styles.metaValue}>
                  {getRuntime(record.task.start_time, record.task.end_time)}
                </span>
              </div>
              {record.task.creation_time && (
                <div className={styles.metaItem}>
                  <span className={styles.metaLabel}>Created</span>
                  <span className={styles.metaValue}>
                    {new Date(record.task.creation_time).toLocaleString()}
                  </span>
                </div>
              )}
            </div>
          </div>

          {/* Tab bar */}
          <Tabs tabs={TABS} activeTab={activeTab} onTabChange={setActiveTab} />

          {/* Tab content */}
          {activeTab === "logs" && (
            <JobLogViewer jobId={record.job_id} status={record.task.status} />
          )}
          {activeTab === "settings" && <JobSettings task={record.task} />}
        </>
      )}
    </div>
  );
}
