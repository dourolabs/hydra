import { useState, useEffect, useCallback, useRef } from "react";
import { LogViewer, Spinner } from "@hydra/ui";
import { useSessionLogs } from "./useSessionLogs";
import styles from "./SessionLogViewer.module.css";

interface SessionLogViewerProps {
  sessionId: string;
  /** Current session status — determines streaming vs snapshot mode. */
  status: string;
}

/** Statuses that indicate the session is still running and should stream. */
const STREAMING_STATUSES = new Set(["created", "pending", "running"]);

export function SessionLogViewer({ sessionId, status }: SessionLogViewerProps) {
  const isStreaming = STREAMING_STATUSES.has(status);

  // For completed sessions: fetch the full log snapshot
  const {
    data: snapshotText,
    isLoading: snapshotLoading,
    error: snapshotError,
  } = useSessionLogs(sessionId, !isStreaming);

  // For running sessions: stream logs via SSE
  const [streamLines, setStreamLines] = useState<string[]>([]);
  const [streamError, setStreamError] = useState<string | null>(null);
  const [streamConnected, setStreamConnected] = useState(false);
  const eventSourceRef = useRef<EventSource | null>(null);

  // Follow output toggle state (only relevant when streaming)
  const [followOutput, setFollowOutput] = useState(true);

  const cleanup = useCallback(() => {
    if (eventSourceRef.current) {
      eventSourceRef.current.close();
      eventSourceRef.current = null;
    }
  }, []);

  const openLogStream = useCallback(() => {
    cleanup();

    const es = new EventSource(
      `/api/v1/sessions/${encodeURIComponent(sessionId)}/logs?watch=true`,
    );
    eventSourceRef.current = es;

    es.onopen = () => {
      setStreamConnected(true);
      setStreamError(null);
    };

    es.onmessage = (event) => {
      const chunk = event.data as string;
      if (chunk) {
        const newLines = chunk.split("\n");
        setStreamLines((prev) => [...prev, ...newLines]);
      }
    };

    es.onerror = () => {
      // EventSource will automatically try to reconnect.
      // If the connection is fully closed (readyState === CLOSED), report an error.
      if (es.readyState === EventSource.CLOSED) {
        setStreamError("Log stream connection lost.");
        setStreamConnected(false);
      }
    };
  }, [sessionId, cleanup]);

  useEffect(() => {
    if (!isStreaming) {
      cleanup();
      return;
    }

    openLogStream();

    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        setStreamLines([]);
        openLogStream();
      }
    };

    document.addEventListener("visibilitychange", handleVisibilityChange);

    return () => {
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      cleanup();
    };
  }, [isStreaming, openLogStream, cleanup]);

  const handleAutoScrollChange = useCallback((isAutoScrolling: boolean) => {
    setFollowOutput(isAutoScrolling);
  }, []);

  const handleFollowToggle = useCallback(() => {
    setFollowOutput((prev) => !prev);
  }, []);

  // Parse snapshot text into lines
  const snapshotLines = snapshotText ? snapshotText.split("\n") : [];

  const lines = isStreaming ? streamLines : snapshotLines;
  const isLoading = !isStreaming && snapshotLoading;
  const error = isStreaming ? streamError : snapshotError;

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
          {streamConnected ? "Streaming logs\u2026" : "Connecting\u2026"}
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
