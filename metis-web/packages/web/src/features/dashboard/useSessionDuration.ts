import { useEffect, useMemo, useState } from "react";
import type { SessionSummaryRecord } from "@metis/api";
import { formatDuration } from "../../utils/time";

interface SessionDuration {
  durationText: string;
  isRunning: boolean;
}

export function useSessionDuration(sessions: SessionSummaryRecord[] | undefined): SessionDuration {
  const runningSession = useMemo(
    () => sessions?.find((s) => s.session.status === "running" || s.session.status === "pending"),
    [sessions],
  );

  const lastFinishedSession = useMemo(() => {
    if (runningSession || !sessions) return undefined;
    return sessions
      .filter((s) => s.session.status === "complete" || s.session.status === "failed")
      .sort((a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime())[0];
  }, [sessions, runningSession]);

  const [elapsed, setElapsed] = useState(() => {
    if (!runningSession?.session.start_time) return 0;
    return Date.now() - new Date(runningSession.session.start_time).getTime();
  });

  useEffect(() => {
    if (!runningSession?.session.start_time) return;
    setElapsed(Date.now() - new Date(runningSession.session.start_time).getTime());
    const id = setInterval(() => {
      setElapsed(Date.now() - new Date(runningSession.session.start_time!).getTime());
    }, 1000);
    return () => clearInterval(id);
  }, [runningSession]);

  if (runningSession) {
    return { durationText: formatDuration(elapsed), isRunning: true };
  }

  if (lastFinishedSession?.session.start_time && lastFinishedSession.session.end_time) {
    return {
      durationText: formatDuration(
        new Date(lastFinishedSession.session.end_time).getTime() - new Date(lastFinishedSession.session.start_time).getTime(),
      ),
      isRunning: false,
    };
  }

  return { durationText: "\u2014", isRunning: false };
}
