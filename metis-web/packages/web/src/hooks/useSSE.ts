import { useEffect, useRef, useState, useCallback } from "react";
import { useQueryClient } from "@tanstack/react-query";
import type { EntityEventData } from "@metis/api";

export type SSEConnectionState = "connecting" | "connected" | "disconnected";

const ENTITY_EVENT_TYPES = [
  "issue_created",
  "issue_updated",
  "issue_deleted",
  "patch_created",
  "patch_updated",
  "patch_deleted",
  "job_created",
  "job_updated",
] as const;

const MAX_BACKOFF_MS = 30_000;
const BASE_BACKOFF_MS = 1_000;

/**
 * SSE hook that connects to the BFF /api/v1/events endpoint, listens for
 * entity mutation events, and invalidates React Query caches to trigger
 * refetches. Handles automatic reconnection with exponential backoff.
 */
export function useSSE(): SSEConnectionState {
  const [state, setState] = useState<SSEConnectionState>("disconnected");
  const queryClient = useQueryClient();
  const retriesRef = useRef(0);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const esRef = useRef<EventSource | null>(null);

  const invalidateForEvent = useCallback(
    (eventType: string, data: EntityEventData) => {
      const { entity_type, entity_id } = data;

      if (entity_type === "issue" || eventType.startsWith("issue_")) {
        queryClient.invalidateQueries({ queryKey: ["issues"] });
        queryClient.invalidateQueries({ queryKey: ["issue", entity_id] });
      } else if (entity_type === "job" || eventType.startsWith("job_")) {
        // Jobs are keyed by parent issue, but we don't know the parent from
        // the event payload alone. Invalidate all job queries so any visible
        // job list refetches.
        queryClient.invalidateQueries({ queryKey: ["jobs"] });
      } else if (entity_type === "patch" || eventType.startsWith("patch_")) {
        queryClient.invalidateQueries({ queryKey: ["patch", entity_id] });
        // Also invalidate the issues list since patches are shown on issue detail
        queryClient.invalidateQueries({ queryKey: ["issues"] });
      }
    },
    [queryClient],
  );

  const invalidateAll = useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["issues"] });
    queryClient.invalidateQueries({ queryKey: ["jobs"] });
    queryClient.invalidateQueries({ queryKey: ["patch"] });
  }, [queryClient]);

  const connect = useCallback(() => {
    // Clean up previous connection
    if (esRef.current) {
      esRef.current.close();
      esRef.current = null;
    }

    setState("connecting");

    const es = new EventSource(
      "/api/v1/events?types=issues,jobs,patches",
    );
    esRef.current = es;

    es.onopen = () => {
      setState("connected");
      retriesRef.current = 0;
    };

    // Entity mutation events
    for (const eventType of ENTITY_EVENT_TYPES) {
      es.addEventListener(eventType, (e: MessageEvent) => {
        try {
          const data: EntityEventData = JSON.parse(e.data);
          invalidateForEvent(eventType, data);
        } catch {
          // Ignore malformed events
        }
      });
    }

    // Snapshot event on initial connection — nothing to do, data is current
    es.addEventListener("snapshot", () => {
      // Data is current as of connection. No action needed.
    });

    // Resync event — client has fallen behind, refetch everything
    es.addEventListener("resync", () => {
      invalidateAll();
    });

    // Heartbeat — keep-alive, no action needed
    es.addEventListener("heartbeat", () => {
      // No-op: confirms connection is alive
    });

    es.onerror = () => {
      es.close();
      esRef.current = null;
      setState("disconnected");

      // Reconnect with exponential backoff
      const delay = Math.min(
        BASE_BACKOFF_MS * 2 ** retriesRef.current,
        MAX_BACKOFF_MS,
      );
      retriesRef.current += 1;
      timerRef.current = setTimeout(connect, delay);
    };
  }, [invalidateForEvent, invalidateAll]);

  useEffect(() => {
    connect();

    return () => {
      if (esRef.current) {
        esRef.current.close();
        esRef.current = null;
      }
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [connect]);

  return state;
}
