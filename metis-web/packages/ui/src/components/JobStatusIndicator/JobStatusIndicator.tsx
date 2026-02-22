import { useEffect, useState } from "react";
import styles from "./JobStatusIndicator.module.css";

export type JobStatus = "created" | "pending" | "running" | "complete" | "failed";

export interface JobSummary {
  jobId: string;
  status: JobStatus;
  startTime?: string | null;
  endTime?: string | null;
}

export interface JobStatusIndicatorProps {
  jobs: JobSummary[];
  onJobClick?: (jobId: string) => void;
  /** Maximum number of past (non-running) dots to show before truncating */
  maxDots?: number;
}

function formatElapsed(ms: number): string {
  const seconds = Math.floor(Math.max(0, ms) / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = seconds % 60;
  if (minutes < 60) return `${minutes}m ${remainingSeconds}s`;
  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  return `${hours}h ${remainingMinutes}m`;
}

function useElapsedTime(startTime: string | null | undefined, active: boolean): string {
  const [elapsed, setElapsed] = useState(() => {
    if (!startTime) return "0s";
    return formatElapsed(Date.now() - new Date(startTime).getTime());
  });

  useEffect(() => {
    if (!active || !startTime) return;
    const update = () => {
      setElapsed(formatElapsed(Date.now() - new Date(startTime).getTime()));
    };
    update();
    const id = setInterval(update, 1000);
    return () => clearInterval(id);
  }, [active, startTime]);

  return elapsed;
}

const statusClass: Record<JobStatus, string> = {
  complete: styles.complete,
  failed: styles.failed,
  running: styles.running,
  created: styles.pending,
  pending: styles.pending,
};

function RunningIndicator({
  job,
  onClick,
}: {
  job: JobSummary;
  onClick?: (jobId: string) => void;
}) {
  const elapsed = useElapsedTime(job.startTime, true);

  return (
    <button
      className={`${styles.runningIndicator}`}
      title={job.jobId}
      onClick={() => onClick?.(job.jobId)}
      type="button"
    >
      <span className={`${styles.dot} ${styles.running}`} />
      <span className={styles.elapsed}>{elapsed}</span>
    </button>
  );
}

export function JobStatusIndicator({
  jobs,
  onJobClick,
  maxDots = 10,
}: JobStatusIndicatorProps) {
  if (jobs.length === 0) return null;

  const runningJobs = jobs.filter((j) => j.status === "running");
  const nonRunningJobs = jobs.filter((j) => j.status !== "running");

  const truncated = nonRunningJobs.length > maxDots;
  const visibleNonRunning = truncated ? nonRunningJobs.slice(-maxDots) : nonRunningJobs;
  const hiddenCount = nonRunningJobs.length - visibleNonRunning.length;

  return (
    <div className={styles.container}>
      {truncated && <span className={styles.countPrefix}>{hiddenCount + visibleNonRunning.length}:</span>}
      {visibleNonRunning.map((job) => (
        <button
          key={job.jobId}
          className={`${styles.dot} ${statusClass[job.status]}`}
          title={job.jobId}
          onClick={() => onJobClick?.(job.jobId)}
          type="button"
        />
      ))}
      {runningJobs.map((job) => (
        <RunningIndicator key={job.jobId} job={job} onClick={onJobClick} />
      ))}
    </div>
  );
}
