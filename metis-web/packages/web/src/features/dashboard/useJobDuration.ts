import { useEffect, useMemo, useState } from "react";
import type { JobSummaryRecord } from "@metis/api";
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
