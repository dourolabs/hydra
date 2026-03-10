import { useEffect, useMemo, useState } from "react";
import type { JobSummaryRecord, JobStatusSummary } from "@metis/api";
import { formatDuration } from "../../utils/time";

interface JobDuration {
  durationText: string;
  isRunning: boolean;
}

export function useJobDuration(jobs: JobSummaryRecord[] | undefined): JobDuration {
  const runningJob = useMemo(
    () => jobs?.find((j) => j.task.status === "running" || j.task.status === "pending"),
    [jobs],
  );

  const lastFinishedJob = useMemo(() => {
    if (runningJob || !jobs) return undefined;
    return jobs
      .filter((j) => j.task.status === "complete" || j.task.status === "failed")
      .sort((a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime())[0];
  }, [jobs, runningJob]);

  const [elapsed, setElapsed] = useState(() => {
    if (!runningJob?.task.start_time) return 0;
    return Date.now() - new Date(runningJob.task.start_time).getTime();
  });

  useEffect(() => {
    if (!runningJob?.task.start_time) return;
    setElapsed(Date.now() - new Date(runningJob.task.start_time).getTime());
    const id = setInterval(() => {
      setElapsed(Date.now() - new Date(runningJob.task.start_time!).getTime());
    }, 1000);
    return () => clearInterval(id);
  }, [runningJob]);

  if (runningJob) {
    return { durationText: formatDuration(elapsed), isRunning: true };
  }

  if (lastFinishedJob?.task.start_time && lastFinishedJob.task.end_time) {
    return {
      durationText: formatDuration(
        new Date(lastFinishedJob.task.end_time).getTime() - new Date(lastFinishedJob.task.start_time).getTime(),
      ),
      isRunning: false,
    };
  }

  return { durationText: "\u2014", isRunning: false };
}

/**
 * Compute job duration from an embedded JobStatusSummary (from include=jobs_summary).
 * Avoids needing the full job list for the dashboard.
 */
export function useJobSummaryDuration(summary: JobStatusSummary | null | undefined): {
  durationText: string;
  isRunning: boolean;
} {
  const isRunning = !!(summary && summary.running > 0);
  const startTime = summary?.latest_start_time;
  const endTime = summary?.latest_end_time;

  const [elapsed, setElapsed] = useState(() => {
    if (!isRunning || !startTime) return 0;
    return Date.now() - new Date(startTime).getTime();
  });

  useEffect(() => {
    if (!isRunning || !startTime) return;
    setElapsed(Date.now() - new Date(startTime).getTime());
    const id = setInterval(() => {
      setElapsed(Date.now() - new Date(startTime).getTime());
    }, 1000);
    return () => clearInterval(id);
  }, [isRunning, startTime]);

  if (isRunning && startTime) {
    return { durationText: formatDuration(elapsed), isRunning: true };
  }

  if (startTime && endTime) {
    return {
      durationText: formatDuration(
        new Date(endTime).getTime() - new Date(startTime).getTime(),
      ),
      isRunning: false,
    };
  }

  return { durationText: "\u2014", isRunning: false };
}
