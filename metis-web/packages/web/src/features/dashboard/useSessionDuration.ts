import { useEffect, useMemo, useState } from "react";
import type { JobSummaryRecord } from "@metis/api";
import { formatDuration } from "../../utils/time";

interface SessionDuration {
  durationText: string;
  isRunning: boolean;
}

export function useSessionDuration(sessions: JobSummaryRecord[] | undefined): SessionDuration {
  const runningSession = useMemo(
    () => sessions?.find((s) => s.task.status === "running" || s.task.status === "pending"),
    [sessions],
  );

  const lastFinishedSession = useMemo(() => {
    if (runningSession || !sessions) return undefined;
    return sessions
      .filter((s) => s.task.status === "complete" || s.task.status === "failed")
      .sort((a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime())[0];
  }, [sessions, runningSession]);

  const [elapsed, setElapsed] = useState(() => {
    if (!runningSession?.task.start_time) return 0;
    return Date.now() - new Date(runningSession.task.start_time).getTime();
  });

  useEffect(() => {
    if (!runningSession?.task.start_time) return;
    setElapsed(Date.now() - new Date(runningSession.task.start_time).getTime());
    const id = setInterval(() => {
      setElapsed(Date.now() - new Date(runningSession.task.start_time!).getTime());
    }, 1000);
    return () => clearInterval(id);
  }, [runningSession]);

  if (runningSession) {
    return { durationText: formatDuration(elapsed), isRunning: true };
  }

  if (lastFinishedSession?.task.start_time && lastFinishedSession.task.end_time) {
    return {
      durationText: formatDuration(
        new Date(lastFinishedSession.task.end_time).getTime() - new Date(lastFinishedSession.task.start_time).getTime(),
      ),
      isRunning: false,
    };
  }

  return { durationText: "\u2014", isRunning: false };
}
