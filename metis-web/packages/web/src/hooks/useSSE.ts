import { useEffect, useRef, useState, useCallback } from "react";
import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import type {
  EntityEventData,
  IssueSummaryRecord,
  JobSummaryRecord,
  PatchSummaryRecord,
  DocumentSummaryRecord,
  ListIssuesResponse,
  ListJobsResponse,
  ListPatchesResponse,
  ListDocumentsResponse,
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
  "document_created",
  "document_updated",
  "document_deleted",
  "label_created",
  "label_updated",
  "label_deleted",
] as const;

const MAX_BACKOFF_MS = 30_000;
const BASE_BACKOFF_MS = 1_000;

// ---------------------------------------------------------------------------
// Cache-update helpers — eliminate repeated version-guard & list-upsert logic
// ---------------------------------------------------------------------------

interface VersionedRecord {
  version: number | bigint;
}

/**
 * Version-guarded upsert into an array within a list-response cache entry.
 * Updates in place (with version guard) if the entity already exists, or
 * appends to cover newly-created entities.
 */
function upsertInList<TResp, TItem extends VersionedRecord>(
  qc: QueryClient,
  key: readonly unknown[],
  getItems: (resp: TResp) => TItem[],
  wrapItems: (items: TItem[]) => TResp,
  getId: (item: TItem) => string,
  entityId: string,
  record: TItem,
) {
  qc.setQueriesData<TResp>({ queryKey: key }, (old) => {
    if (!old) return old;
    const arr = getItems(old);
    const idx = arr.findIndex((a) => getId(a) === entityId);
    if (idx >= 0) {
      if (arr[idx].version > record.version) return old;
      const updated = [...arr];
      updated[idx] = record;
      return wrapItems(updated);
    }
    return wrapItems([...arr, record]);
  });
}

/** Remove an entity from an array within a list-response cache entry. */
function removeFromList<TResp, TItem>(
  qc: QueryClient,
  key: readonly unknown[],
  getItems: (resp: TResp) => TItem[],
  wrapItems: (items: TItem[]) => TResp,
  getId: (item: TItem) => string,
  entityId: string,
) {
  qc.setQueriesData<TResp>({ queryKey: key }, (old) => {
    if (!old) return old;
    return wrapItems(getItems(old).filter((a) => getId(a) !== entityId));
  });
}

// Entity-specific accessors for the list-response shapes
const issueList = (r: ListIssuesResponse) => r.issues;
const wrapIssues = (items: IssueSummaryRecord[]): ListIssuesResponse => ({ issues: items });
const issueRecordId = (r: IssueSummaryRecord) => r.issue_id;

const jobList = (r: ListJobsResponse) => r.jobs;
const wrapJobs = (items: JobSummaryRecord[]): ListJobsResponse => ({ jobs: items });
const jobRecordId = (r: JobSummaryRecord) => r.job_id;

const patchList = (r: ListPatchesResponse) => r.patches;
const wrapPatches = (items: PatchSummaryRecord[]): ListPatchesResponse => ({ patches: items });
const patchRecordId = (r: PatchSummaryRecord) => r.patch_id;

const docList = (r: ListDocumentsResponse) => r.documents;
const wrapDocs = (items: DocumentSummaryRecord[]): ListDocumentsResponse => ({ documents: items });
const docRecordId = (r: DocumentSummaryRecord) => r.document_id;

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
        queryClient.invalidateQueries({ queryKey: ["allJobs"] });
      } else if (entity_type === "patch" || eventType.startsWith("patch_")) {
        queryClient.invalidateQueries({ queryKey: ["patches"] });
        queryClient.invalidateQueries({ queryKey: ["patch", entity_id] });
      } else if (entity_type === "document" || eventType.startsWith("document_")) {
        queryClient.invalidateQueries({ queryKey: ["documents"] });
        queryClient.invalidateQueries({ queryKey: ["document", entity_id] });
      } else if (entity_type === "label" || eventType.startsWith("label_")) {
        queryClient.invalidateQueries({ queryKey: ["labels"] });
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
          queryClient.removeQueries({ queryKey: ["issue", entity_id] });
          removeFromList(queryClient, ["issues"], issueList, wrapIssues, issueRecordId, entity_id);
        } else {
          const record = entity as unknown as IssueSummaryRecord;
          queryClient.invalidateQueries({ queryKey: ["issue", entity_id] });
          upsertInList(queryClient, ["issues"], issueList, wrapIssues, issueRecordId, entity_id, record);
          queryClient.invalidateQueries({ queryKey: ["issue", entity_id, "versions"] });
        }
      } else if (entity_type === "job" || eventType.startsWith("job_")) {
        const record = entity as unknown as JobSummaryRecord;
        const spawnedFrom = record.task?.spawned_from;
        const isTerminal = record.task?.status === "complete" || record.task?.status === "failed" || record.task?.status === "unknown";

        // SSE now sends summary records; invalidate the detail cache instead of direct-setting.
        queryClient.invalidateQueries({ queryKey: ["job", entity_id] });

        // allJobs only contains active (non-terminal) jobs. Remove jobs that
        // transition to a terminal status; upsert otherwise.
        if (isTerminal) {
          removeFromList(queryClient, ["allJobs"], jobList, wrapJobs, jobRecordId, entity_id);
        } else {
          upsertInList(queryClient, ["allJobs"], jobList, wrapJobs, jobRecordId, entity_id, record);
        }

        if (spawnedFrom) {
          upsertInList(queryClient, ["jobs", spawnedFrom], jobList, wrapJobs, jobRecordId, entity_id, record);
        } else {
          queryClient.invalidateQueries({ queryKey: ["jobs"] });
        }
      } else if (entity_type === "patch" || eventType.startsWith("patch_")) {
        if (eventType === "patch_deleted") {
          queryClient.removeQueries({ queryKey: ["patch", entity_id] });
          removeFromList(queryClient, ["patches"], patchList, wrapPatches, patchRecordId, entity_id);
        } else {
          const record = entity as unknown as PatchSummaryRecord;
          // SSE now carries summary records — update list cache directly
          upsertInList(queryClient, ["patches"], patchList, wrapPatches, patchRecordId, entity_id, record);
          // Detail cache uses full PatchVersionRecord; invalidate so it refetches
          queryClient.invalidateQueries({ queryKey: ["patch", entity_id] });
        }
      } else if (entity_type === "document" || eventType.startsWith("document_")) {
        if (eventType === "document_deleted") {
          queryClient.removeQueries({ queryKey: ["document", entity_id] });
          removeFromList(queryClient, ["documents"], docList, wrapDocs, docRecordId, entity_id);
        } else {
          const record = entity as unknown as DocumentSummaryRecord;
          // Invalidate the detail cache since SSE now carries summary data only
          queryClient.invalidateQueries({ queryKey: ["document", entity_id] });
          upsertInList(queryClient, ["documents"], docList, wrapDocs, docRecordId, entity_id, record);
        }
      } else if (entity_type === "label" || eventType.startsWith("label_")) {
        queryClient.invalidateQueries({ queryKey: ["labels"] });
      }
    },
    [queryClient, invalidateForEvent],
  );

  const invalidateAll = useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["issues"] });
    queryClient.invalidateQueries({ queryKey: ["jobs"] });
    queryClient.invalidateQueries({ queryKey: ["allJobs"] });
    queryClient.invalidateQueries({ queryKey: ["patches"] });
    queryClient.invalidateQueries({ queryKey: ["documents"] });
    queryClient.invalidateQueries({ queryKey: ["labels"] });
  }, [queryClient]);

  const connect = useCallback(() => {
    // Clean up previous connection
    if (esRef.current) {
      esRef.current.close();
      esRef.current = null;
    }

    setState("connecting");

    const es = new EventSource("/api/v1/events?types=issues,jobs,patches,documents,labels");
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
      const delay = Math.min(BASE_BACKOFF_MS * 2 ** retriesRef.current, MAX_BACKOFF_MS);
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

  // Reconnect and refresh caches when the page becomes visible again
  // (e.g., after mobile suspend or tab switch)
  useEffect(() => {
    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        retriesRef.current = 0;
        if (timerRef.current) {
          clearTimeout(timerRef.current);
          timerRef.current = null;
        }
        invalidateAll();
        connect();
      }
    };

    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [connect, invalidateAll]);

  return state;
}
