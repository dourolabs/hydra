import { useEffect, useMemo, useState } from "react";
import type { SessionSummaryRecord } from "@hydra/api";
import type { RunTimeStatus } from "../../components/Runtime/Runtime";
import { formatDuration } from "../../utils/time";

interface SessionDuration {
  durationText: string;
  isRunning: boolean;
  status: RunTimeStatus;
}

function isRunningStatus(s: SessionSummaryRecord): boolean {
  return s.session.status === "running" || s.session.status === "pending";
}

export function useSessionDuration(sessions: SessionSummaryRecord[] | undefined): SessionDuration {
  const runningSession = useMemo(
    () => sessions?.find(isRunningStatus),
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
    return { durationText: formatDuration(elapsed), isRunning: true, status: "in_progress" };
  }

  if (lastFinishedSession?.session.start_time && lastFinishedSession.session.end_time) {
    const status: RunTimeStatus =
      lastFinishedSession.session.status === "failed" ? "failed" : "idle";
    return {
      durationText: formatDuration(
        new Date(lastFinishedSession.session.end_time).getTime() - new Date(lastFinishedSession.session.start_time).getTime(),
      ),
      isRunning: false,
      status,
    };
  }

  return { durationText: "\u2014", isRunning: false, status: "idle" };
}

/**
 * Runtime for a single session: ticks every second while the session is
 * running, otherwise renders end-start (or now-start if end is missing).
 */
export function useSingleSessionDuration(
  session: SessionSummaryRecord | undefined,
): SessionDuration {
  const running = !!session && isRunningStatus(session);
  const startTime = session?.session.start_time ?? null;
  const endTime = session?.session.end_time ?? null;

  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!running || !startTime) return;
    setNow(Date.now());
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [running, startTime]);

  if (!startTime) return { durationText: "\u2014", isRunning: false, status: "idle" };
  const start = new Date(startTime).getTime();
  const end = endTime ? new Date(endTime).getTime() : now;
  const status: RunTimeStatus = running
    ? "in_progress"
    : session?.session.status === "failed"
      ? "failed"
      : "idle";
  return { durationText: formatDuration(end - start), isRunning: running, status };
}
