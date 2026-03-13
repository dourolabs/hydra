import { useEffect, useState } from "react";
import styles from "./SessionStatusIndicator.module.css";

export type SessionStatus = "created" | "pending" | "running" | "complete" | "failed";

export interface SessionSummary {
  sessionId: string;
  status: SessionStatus;
  startTime?: string | null;
  endTime?: string | null;
}

export interface SessionStatusIndicatorProps {
  sessions: SessionSummary[];
  onSessionClick?: (sessionId: string) => void;
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

const statusClass: Record<SessionStatus, string> = {
  complete: styles.complete,
  failed: styles.failed,
  running: styles.running,
  created: styles.pending,
  pending: styles.pending,
};

function RunningIndicator({
  session,
  onClick,
}: {
  session: SessionSummary;
  onClick?: (sessionId: string) => void;
}) {
  const elapsed = useElapsedTime(session.startTime, true);

  return (
    <button
      className={`${styles.runningIndicator}`}
      title={session.sessionId}
      onClick={() => onClick?.(session.sessionId)}
      type="button"
    >
      <span className={`${styles.dot} ${styles.running}`} />
      <span className={styles.elapsed}>{elapsed}</span>
    </button>
  );
}

export function SessionStatusIndicator({
  sessions,
  onSessionClick,
  maxDots = 10,
}: SessionStatusIndicatorProps) {
  if (sessions.length === 0) return null;

  const runningSessions = sessions.filter((s) => s.status === "running");
  const nonRunningSessions = sessions.filter((s) => s.status !== "running");

  const truncated = nonRunningSessions.length > maxDots;
  const visibleNonRunning = truncated ? nonRunningSessions.slice(-maxDots) : nonRunningSessions;
  const hiddenCount = nonRunningSessions.length - visibleNonRunning.length;

  return (
    <div className={styles.container}>
      {truncated && <span className={styles.countPrefix}>{hiddenCount + visibleNonRunning.length}:</span>}
      {visibleNonRunning.map((session) => (
        <button
          key={session.sessionId}
          className={`${styles.dot} ${statusClass[session.status]}`}
          title={session.sessionId}
          onClick={() => onSessionClick?.(session.sessionId)}
          type="button"
        />
      ))}
      {runningSessions.map((session) => (
        <RunningIndicator key={session.sessionId} session={session} onClick={onSessionClick} />
      ))}
    </div>
  );
}
