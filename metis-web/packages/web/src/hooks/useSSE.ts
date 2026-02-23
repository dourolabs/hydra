import { useEffect, useRef, useState, useCallback } from "react";
import { useQueryClient, type QueryClient, type InfiniteData } from "@tanstack/react-query";
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
 * Version-guarded upsert into an array within a regular (non-infinite) query
 * cache entry. Updates in place (with version guard) if the entity already
 * exists, or appends to cover newly-created entities.
 */
function upsertInList<TResp, TItem extends VersionedRecord>(
  qc: QueryClient,
  key: readonly unknown[],
  getItems: (resp: TResp) => TItem[],
  setItems: (resp: TResp, items: TItem[]) => TResp,
  getId: (item: TItem) => string,
  entityId: string,
  record: TItem,
) {
  qc.setQueryData<TResp>(key, (old) => {
    if (!old) return old;
    const arr = getItems(old);
    const idx = arr.findIndex((a) => getId(a) === entityId);
    if (idx >= 0) {
      if (arr[idx].version > record.version) return old;
      const updated = [...arr];
      updated[idx] = record;
      return setItems(old, updated);
    }
    return setItems(old, [...arr, record]);
  });
}


/**
 * Version-guarded upsert into an infinite query cache. Scans all pages for the
 * entity; updates in place if found, otherwise prepends to the first page.
 */
function upsertInInfiniteList<TResp, TItem extends VersionedRecord>(
  qc: QueryClient,
  key: readonly unknown[],
  getItems: (resp: TResp) => TItem[],
  setItems: (resp: TResp, items: TItem[]) => TResp,
  getId: (item: TItem) => string,
  entityId: string,
  record: TItem,
) {
  qc.setQueryData<InfiniteData<TResp>>(key, (old) => {
    if (!old || old.pages.length === 0) return old;

    for (let i = 0; i < old.pages.length; i++) {
      const items = getItems(old.pages[i]);
      const idx = items.findIndex((a) => getId(a) === entityId);
      if (idx >= 0) {
        if (items[idx].version > record.version) return old;
        const newPages = [...old.pages];
        const newItems = [...items];
        newItems[idx] = record;
        newPages[i] = setItems(old.pages[i], newItems);
        return { ...old, pages: newPages };
      }
    }

    // Not found — prepend to first page
    const newPages = [...old.pages];
    newPages[0] = setItems(old.pages[0], [record, ...getItems(old.pages[0])]);
    return { ...old, pages: newPages };
  });
}

/** Remove an entity from an infinite query cache (scans all pages). */
function removeFromInfiniteList<TResp, TItem>(
  qc: QueryClient,
  key: readonly unknown[],
  getItems: (resp: TResp) => TItem[],
  setItems: (resp: TResp, items: TItem[]) => TResp,
  getId: (item: TItem) => string,
  entityId: string,
) {
  qc.setQueryData<InfiniteData<TResp>>(key, (old) => {
    if (!old || old.pages.length === 0) return old;
    let changed = false;
    const newPages = old.pages.map((page) => {
      const items = getItems(page);
      const filtered = items.filter((a) => getId(a) !== entityId);
      if (filtered.length < items.length) {
        changed = true;
        return setItems(page, filtered);
      }
      return page;
    });
    return changed ? { ...old, pages: newPages } : old;
  });
}

// Entity-specific accessors for the list-response shapes
const issueList = (r: ListIssuesResponse) => r.issues;
const setIssueItems = (r: ListIssuesResponse, items: IssueSummaryRecord[]): ListIssuesResponse => ({ ...r, issues: items });
const issueRecordId = (r: IssueSummaryRecord) => r.issue_id;

const jobList = (r: ListJobsResponse) => r.jobs;
const setJobItems = (r: ListJobsResponse, items: JobSummaryRecord[]): ListJobsResponse => ({ ...r, jobs: items });
const jobRecordId = (r: JobSummaryRecord) => r.job_id;

const patchList = (r: ListPatchesResponse) => r.patches;
const setPatchItems = (r: ListPatchesResponse, items: PatchSummaryRecord[]): ListPatchesResponse => ({ ...r, patches: items });
const patchRecordId = (r: PatchSummaryRecord) => r.patch_id;

const docList = (r: ListDocumentsResponse) => r.documents;
const setDocItems = (r: ListDocumentsResponse, items: DocumentSummaryRecord[]): ListDocumentsResponse => ({ ...r, documents: items });
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
          removeFromInfiniteList(queryClient, ["issues"], issueList, setIssueItems, issueRecordId, entity_id);
        } else {
          const record = entity as unknown as IssueSummaryRecord;
          queryClient.invalidateQueries({ queryKey: ["issue", entity_id] });
          upsertInInfiniteList(queryClient, ["issues"], issueList, setIssueItems, issueRecordId, entity_id, record);
          queryClient.invalidateQueries({ queryKey: ["issue", entity_id, "versions"] });
        }
      } else if (entity_type === "job" || eventType.startsWith("job_")) {
        const record = entity as unknown as JobSummaryRecord;
        const spawnedFrom = record.task?.spawned_from;

        // SSE now sends summary records; invalidate the detail cache instead of direct-setting.
        queryClient.invalidateQueries({ queryKey: ["job", entity_id] });
        upsertInList(queryClient, ["allJobs"], jobList, setJobItems, jobRecordId, entity_id, record);

        if (spawnedFrom) {
          upsertInList(queryClient, ["jobs", spawnedFrom], jobList, setJobItems, jobRecordId, entity_id, record);
        } else {
          queryClient.invalidateQueries({ queryKey: ["jobs"] });
        }
      } else if (entity_type === "patch" || eventType.startsWith("patch_")) {
        if (eventType === "patch_deleted") {
          queryClient.removeQueries({ queryKey: ["patch", entity_id] });
          removeFromInfiniteList(queryClient, ["patches"], patchList, setPatchItems, patchRecordId, entity_id);
        } else {
          const record = entity as unknown as PatchSummaryRecord;
          // SSE now carries summary records — update list cache directly
          upsertInInfiniteList(queryClient, ["patches"], patchList, setPatchItems, patchRecordId, entity_id, record);
          // Detail cache uses full PatchVersionRecord; invalidate so it refetches
          queryClient.invalidateQueries({ queryKey: ["patch", entity_id] });
        }
      } else if (entity_type === "document" || eventType.startsWith("document_")) {
        if (eventType === "document_deleted") {
          queryClient.removeQueries({ queryKey: ["document", entity_id] });
          removeFromInfiniteList(queryClient, ["documents"], docList, setDocItems, docRecordId, entity_id);
        } else {
          const record = entity as unknown as DocumentSummaryRecord;
          // Invalidate the detail cache since SSE now carries summary data only
          queryClient.invalidateQueries({ queryKey: ["document", entity_id] });
          upsertInInfiniteList(queryClient, ["documents"], docList, setDocItems, docRecordId, entity_id, record);
        }
      }
    },
    [queryClient, invalidateForEvent],
  );

  const invalidateAll = useCallback(() => {
    queryClient.invalidateQueries({ queryKey: ["issues"] });
    queryClient.invalidateQueries({ queryKey: ["jobs"] });
    queryClient.invalidateQueries({ queryKey: ["allJobs"] });
    queryClient.invalidateQueries({ queryKey: ["patch"] });
    queryClient.invalidateQueries({ queryKey: ["patches"] });
    queryClient.invalidateQueries({ queryKey: ["documents"] });
    queryClient.invalidateQueries({ queryKey: ["document"] });
  }, [queryClient]);

  const connect = useCallback(() => {
    // Clean up previous connection
    if (esRef.current) {
      esRef.current.close();
      esRef.current = null;
    }

    setState("connecting");

    const es = new EventSource("/api/v1/events?types=issues,jobs,patches,documents");
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
