import { useState, useEffect, useCallback, useRef } from "react";
import { LogViewer, Spinner } from "@metis/ui";
import { useJobLogs } from "./useJobLogs";
import { streamJobLogs } from "../../api/jobs";
import styles from "./JobLogViewer.module.css";

interface JobLogViewerProps {
  jobId: string;
  /** Current job status — determines streaming vs snapshot mode. */
  status: string;
}

/** Statuses that indicate the job is still running and should stream. */
const STREAMING_STATUSES = new Set(["created", "pending", "running"]);

export function JobLogViewer({ jobId, status }: JobLogViewerProps) {
  const isStreaming = STREAMING_STATUSES.has(status);

  // For completed jobs: fetch the full log snapshot
  const {
    data: snapshotText,
    isLoading: snapshotLoading,
    error: snapshotError,
  } = useJobLogs(jobId, !isStreaming);

  // For running jobs: stream logs via SSE
  const [streamLines, setStreamLines] = useState<string[]>([]);
  const [streamError, setStreamError] = useState<string | null>(null);
  const [streamConnected, setStreamConnected] = useState(false);
  const eventSourceRef = useRef<EventSource | null>(null);

  const cleanup = useCallback(() => {
    if (eventSourceRef.current) {
      eventSourceRef.current.close();
      eventSourceRef.current = null;
    }
  }, []);

  useEffect(() => {
    if (!isStreaming) {
      cleanup();
      return;
    }

    const es = streamJobLogs(jobId);
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

    return cleanup;
  }, [jobId, isStreaming, cleanup]);

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
        </div>
      )}
      <LogViewer lines={lines} autoScroll={isStreaming} className={styles.logViewer} />
    </div>
  );
}
