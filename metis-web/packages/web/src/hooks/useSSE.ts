import { useEffect, useRef, useState, useCallback } from "react";
import { useQueryClient } from "@tanstack/react-query";
import type {
  EntityEventData,
  IssueVersionRecord,
  JobVersionRecord,
  PatchVersionRecord,
  ListIssuesResponse,
  ListJobsResponse,
} from "@metis/api";

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
 * entity mutation events, and updates React Query caches. When entity data
 * is included in the event payload, the cache is updated directly to avoid
 * unnecessary HTTP re-fetches. Falls back to cache invalidation when entity
 * data is not available (backward compatibility).
 */
export function useSSE(): SSEConnectionState {
  const [state, setState] = useState<SSEConnectionState>("disconnected");
  const queryClient = useQueryClient();
  const retriesRef = useRef(0);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const esRef = useRef<EventSource | null>(null);

  /** Fallback: invalidate caches to trigger refetches (used when no entity data). */
  const invalidateForEvent = useCallback(
    (eventType: string, data: EntityEventData) => {
      const { entity_type, entity_id } = data;

      if (entity_type === "issue" || eventType.startsWith("issue_")) {
        queryClient.invalidateQueries({ queryKey: ["issues"] });
        queryClient.invalidateQueries({ queryKey: ["issue", entity_id] });
      } else if (entity_type === "job" || eventType.startsWith("job_")) {
        queryClient.invalidateQueries({ queryKey: ["jobs"] });
      } else if (entity_type === "patch" || eventType.startsWith("patch_")) {
        queryClient.invalidateQueries({ queryKey: ["patch", entity_id] });
      }
    },
    [queryClient],
  );

  /** Apply a direct cache update from SSE entity data, or fall back to invalidation. */
  const handleEntityEvent = useCallback(
    (eventType: string, data: EntityEventData) => {
      const { entity_type, entity_id, entity } = data;

      // Fall back to invalidation if no entity data is provided
      if (entity == null) {
        invalidateForEvent(eventType, data);
        return;
      }

      if (entity_type === "issue" || eventType.startsWith("issue_")) {
        if (eventType === "issue_deleted") {
          // removeQueries with prefix matching removes both ["issue", id]
          // and ["issue", id, "versions"]
          queryClient.removeQueries({ queryKey: ["issue", entity_id] });
          queryClient.setQueryData<ListIssuesResponse>(["issues"], (old) => {
            if (!old) return old;
            return {
              issues: old.issues.filter((i) => i.issue_id !== entity_id),
            };
          });
        } else {
          const record = entity as unknown as IssueVersionRecord;
          // Update individual issue cache with version guard
          queryClient.setQueryData<IssueVersionRecord>(
            ["issue", entity_id],
            (old) => {
              if (old && old.version > record.version) return old;
              return record;
            },
          );
          // Update the issues list cache
          queryClient.setQueryData<ListIssuesResponse>(["issues"], (old) => {
            if (!old) return old;
            const idx = old.issues.findIndex(
              (i) => i.issue_id === entity_id,
            );
            if (idx >= 0) {
              if (old.issues[idx].version > record.version) return old;
              const updated = [...old.issues];
              updated[idx] = record;
              return { issues: updated };
            }
            // Entry not in list — append (covers created events and missed creates)
            return { issues: [...old.issues, record] };
          });
          // Invalidate the version history since we only have the latest version
          queryClient.invalidateQueries({
            queryKey: ["issue", entity_id, "versions"],
          });
        }
      } else if (entity_type === "job" || eventType.startsWith("job_")) {
        const record = entity as unknown as JobVersionRecord;
        const spawnedFrom = record.task?.spawned_from;

        // Update individual job cache
        queryClient.setQueryData<JobVersionRecord>(
          ["job", entity_id],
          (old) => {
            if (old && old.version > record.version) return old;
            return record;
          },
        );

        if (spawnedFrom) {
          // Update the jobs-by-issue list cache
          queryClient.setQueryData<ListJobsResponse>(
            ["jobs", spawnedFrom],
            (old) => {
              if (!old) return old;
              const idx = old.jobs.findIndex(
                (j) => j.job_id === entity_id,
              );
              if (idx >= 0) {
                if (old.jobs[idx].version > record.version) return old;
                const updated = [...old.jobs];
                updated[idx] = record;
                return { jobs: updated };
              }
              return { jobs: [...old.jobs, record] };
            },
          );
        } else {
          // No spawned_from — fall back to broad invalidation
          queryClient.invalidateQueries({ queryKey: ["jobs"] });
        }
      } else if (entity_type === "patch" || eventType.startsWith("patch_")) {
        if (eventType === "patch_deleted") {
          queryClient.removeQueries({ queryKey: ["patch", entity_id] });
        } else {
          const record = entity as unknown as PatchVersionRecord;
          queryClient.setQueryData<PatchVersionRecord>(
            ["patch", entity_id],
            (old) => {
              if (old && old.version > record.version) return old;
              return record;
            },
          );
        }
      }
    },
    [queryClient, invalidateForEvent],
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
          handleEntityEvent(eventType, data);
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
  }, [handleEntityEvent, invalidateAll]);

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
