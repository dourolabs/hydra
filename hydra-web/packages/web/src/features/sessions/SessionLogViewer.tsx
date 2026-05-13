import { useState, useEffect, useCallback, useRef } from "react";
import { LogViewer, Spinner } from "@hydra/ui";
import { useSessionLogs } from "./useSessionLogs";
import { splitLogLines } from "./splitLogLines";
import { sessionLogRegistry } from "../../hooks/sessionLogRegistry";
import styles from "./SessionLogViewer.module.css";

interface SessionLogViewerProps {
  sessionId: string;
  /** Current session status — determines streaming vs snapshot mode. */
  status: string;
}

/** Statuses that indicate the session is still running and should stream. */
const STREAMING_STATUSES = new Set(["created", "pending", "running"]);

/** Maximum number of lines to keep in the streaming buffer. */
const MAX_STREAM_LINES = 50_000;

export function SessionLogViewer({ sessionId, status }: SessionLogViewerProps) {
  const isStreaming = STREAMING_STATUSES.has(status);

  // For completed sessions: fetch the full log snapshot
  const {
    data: snapshotText,
    isLoading: snapshotLoading,
    error: snapshotError,
  } = useSessionLogs(sessionId, !isStreaming);

  // For running sessions: stream logs over the global /api/v1/events SSE via
  // the session-log registry rather than opening a per-session EventSource.
  // This keeps the browser's per-origin HTTP/1.1 connection cap from being
  // saturated by long-lived log streams.
  const [streamLines, setStreamLines] = useState<string[]>([]);

  // RAF batching: accumulate lines in a ref, flush on animation frame.
  const pendingLinesRef = useRef<string[]>([]);
  const rafIdRef = useRef<number | null>(null);

  const flushPendingLines = useCallback(() => {
    rafIdRef.current = null;
    const pending = pendingLinesRef.current;
    if (pending.length === 0) return;
    pendingLinesRef.current = [];

    setStreamLines((prev) => {
      const combined =
        prev.length + pending.length > MAX_STREAM_LINES
          ? [...prev, ...pending].slice(-MAX_STREAM_LINES)
          : [...prev, ...pending];
      return combined;
    });
  }, []);

  // Follow output toggle state (only relevant when streaming)
  const [followOutput, setFollowOutput] = useState(true);

  useEffect(() => {
    if (!isStreaming) {
      pendingLinesRef.current = [];
      if (rafIdRef.current !== null) {
        cancelAnimationFrame(rafIdRef.current);
        rafIdRef.current = null;
      }
      return;
    }

    const handleChunk = (chunk: string) => {
      if (!chunk) return;
      const newLines = splitLogLines(chunk);
      pendingLinesRef.current.push(...newLines);
      if (rafIdRef.current === null) {
        rafIdRef.current = requestAnimationFrame(flushPendingLines);
      }
    };

    const unsubscribe = sessionLogRegistry.subscribe(sessionId, handleChunk);

    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        // The global SSE handles reconnect; clear the buffer so we don't show
        // stale chunks while the global stream catches back up.
        setStreamLines([]);
      }
    };

    document.addEventListener("visibilitychange", handleVisibilityChange);

    return () => {
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      unsubscribe();
      if (rafIdRef.current !== null) {
        cancelAnimationFrame(rafIdRef.current);
        rafIdRef.current = null;
      }
      pendingLinesRef.current = [];
    };
  }, [isStreaming, sessionId, flushPendingLines]);

  const handleAutoScrollChange = useCallback((isAutoScrolling: boolean) => {
    setFollowOutput(isAutoScrolling);
  }, []);

  const handleFollowToggle = useCallback(() => {
    setFollowOutput((prev) => !prev);
  }, []);

  // Parse snapshot text into lines
  const snapshotLines = snapshotText ? splitLogLines(snapshotText) : [];

  const lines = isStreaming ? streamLines : snapshotLines;
  const isLoading = !isStreaming && snapshotLoading;
  const error = isStreaming ? null : snapshotError;

  if (isLoading) {
    return (
      <div className={styles.center}>
        <Spinner size="md" />
      </div>
    );
  }

  if (error) {
    return (
      <div className={styles.errorContainer}>
        <p className={styles.error}>
          {error instanceof Error ? error.message : String(error)}
        </p>
      </div>
    );
  }

  return (
    <div className={styles.viewer}>
      {isStreaming && (
        <div className={styles.streamingIndicator}>
          <span className={styles.dot} />
          Streaming logs&hellip;
          <label className={styles.followToggle}>
            <input
              type="checkbox"
              checked={followOutput}
              onChange={handleFollowToggle}
              className={styles.followCheckbox}
            />
            Follow output
          </label>
        </div>
      )}
      <LogViewer
        lines={lines}
        autoScroll={isStreaming && followOutput}
        className={styles.logViewer}
        onAutoScrollChange={handleAutoScrollChange}
      />
    </div>
  );
}
