import { useEffect, useRef, useState, useCallback } from "react";
import { useQueryClient, type QueryClient, type InfiniteData } from "@tanstack/react-query";
import type {
  EntityEventData,
  IssueSummaryRecord,
  JobSummaryRecord,
  PatchSummaryRecord,
  DocumentSummaryRecord,
  JobStatusSummary,
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

// ---------------------------------------------------------------------------
// Paginated (infinite query) in-place update helpers
// ---------------------------------------------------------------------------

type PaginatedIssuesData = InfiniteData<ListIssuesResponse>;

/**
 * Update an issue record in-place within all paginated issue queries.
 * Walks every loaded page across all matching query keys and performs a
 * version-guarded replacement.  Returns true if the entity was found and
 * updated in at least one query.
 */
function upsertInPaginatedIssues(
  qc: QueryClient,
  entityId: string,
  record: IssueSummaryRecord,
): boolean {
  let found = false;
  qc.setQueriesData<PaginatedIssuesData>(
    { queryKey: ["paginatedIssues"] },
    (old) => {
      if (!old) return old;
      let changed = false;
      const pages = old.pages.map((page) => {
        const idx = page.issues.findIndex((i: IssueSummaryRecord) => i.issue_id === entityId);
        if (idx < 0) return page;
        if (page.issues[idx].version > record.version) return page;
        changed = true;
        const updated = [...page.issues];
        updated[idx] = record;
        return { ...page, issues: updated };
      });
      if (!changed) return old;
      found = true;
      return { ...old, pages };
    },
  );
  return found;
}

/**
 * Remove an issue from all paginated issue queries.
 */
function removeFromPaginatedIssues(
  qc: QueryClient,
  entityId: string,
): void {
  qc.setQueriesData<PaginatedIssuesData>(
    { queryKey: ["paginatedIssues"] },
    (old) => {
      if (!old) return old;
      let changed = false;
      const pages = old.pages.map((page) => {
        const filtered = page.issues.filter((i: IssueSummaryRecord) => i.issue_id !== entityId);
        if (filtered.length !== page.issues.length) changed = true;
        return filtered.length === page.issues.length ? page : { ...page, issues: filtered };
      });
      if (!changed) return old;
      return { ...old, pages };
    },
  );
}

/**
 * Update the embedded jobs_summary for a specific issue across all paginated
 * issue queries.  Uses setQueriesData to update the job summary in-place
 * within loaded pages so that a full re-fetch is not required.
 */
function updateJobSummaryInPaginatedIssues(
  qc: QueryClient,
  issueId: string,
  jobRecord: JobSummaryRecord,
): boolean {
  let found = false;
  qc.setQueriesData<PaginatedIssuesData>(
    { queryKey: ["paginatedIssues"] },
    (old) => {
      if (!old) return old;
      let changed = false;
      const pages = old.pages.map((page) => {
        const idx = page.issues.findIndex((i: IssueSummaryRecord) => i.issue_id === issueId);
        if (idx < 0) return page;
        changed = true;
        const issue = page.issues[idx];
        const prev = issue.jobs_summary;
        // Incrementally update the summary from the job event
        const jobStatus = jobRecord.task?.status;
        const isRunning = jobStatus === "running" || jobStatus === "pending";
        const isFailed = jobStatus === "failed";
        const summary: JobStatusSummary = prev
          ? { ...prev }
          : {
              total: 0,
              running: 0,
              failed: 0,
              latest_job_id: null,
              latest_job_status: null,
              latest_start_time: null,
              latest_end_time: null,
            };
        // Update latest job info
        summary.latest_job_id = jobRecord.job_id;
        summary.latest_job_status = (jobStatus as JobStatusSummary["latest_job_status"]) ?? null;
        summary.latest_start_time = jobRecord.task?.start_time ?? summary.latest_start_time;
        summary.latest_end_time = jobRecord.task?.end_time ?? summary.latest_end_time;
        // We can't perfectly track running/failed counts from a single event,
        // but we can mark the summary as stale. For now, do a best-effort
        // update: if the latest job is running, ensure running >= 1.
        if (isRunning && summary.running === 0) summary.running = 1;
        if (isFailed && summary.failed === 0) summary.failed = 1;
        const updated = [...page.issues];
        updated[idx] = { ...issue, jobs_summary: summary };
        return { ...page, issues: updated };
      });
      if (!changed) return old;
      found = true;
      return { ...old, pages };
    },
  );
  return found;
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
        queryClient.invalidateQueries({ queryKey: ["paginatedIssues"] });
        queryClient.invalidateQueries({ queryKey: ["issue", entity_id] });
      } else if (entity_type === "job" || eventType.startsWith("job_")) {
        queryClient.invalidateQueries({ queryKey: ["jobs"] });
        queryClient.invalidateQueries({ queryKey: ["allJobs"] });
        // Invalidate paginated issues since embedded jobs_summary may have changed
        queryClient.invalidateQueries({ queryKey: ["paginatedIssues"] });
      } else if (entity_type === "patch" || eventType.startsWith("patch_")) {
        queryClient.invalidateQueries({ queryKey: ["patches"] });
        queryClient.invalidateQueries({ queryKey: ["patch", entity_id] });
      } else if (entity_type === "document" || eventType.startsWith("document_")) {
        queryClient.invalidateQueries({ queryKey: ["documents"] });
        queryClient.invalidateQueries({ queryKey: ["paginatedDocuments"] });
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
          removeFromPaginatedIssues(queryClient, entity_id);
        } else {
          const record = entity as unknown as IssueSummaryRecord;
          queryClient.invalidateQueries({ queryKey: ["issue", entity_id] });
          upsertInList(queryClient, ["issues"], issueList, wrapIssues, issueRecordId, entity_id, record);
          queryClient.invalidateQueries({ queryKey: ["issue", entity_id, "versions"] });
          // Update in-place within paginated pages; only invalidate if the
          // entity was not found (e.g. a newly created issue that may belong
          // to an earlier page).
          const updated = upsertInPaginatedIssues(queryClient, entity_id, record);
          if (!updated) {
            queryClient.invalidateQueries({ queryKey: ["paginatedIssues"] });
          }
        }
      } else if (entity_type === "job" || eventType.startsWith("job_")) {
        const record = entity as unknown as JobSummaryRecord;
        const spawnedFrom = record.task?.spawned_from;

        // SSE now sends summary records; invalidate the detail cache instead of direct-setting.
        queryClient.invalidateQueries({ queryKey: ["job", entity_id] });
        upsertInList(queryClient, ["allJobs"], jobList, wrapJobs, jobRecordId, entity_id, record);

        if (spawnedFrom) {
          upsertInList(queryClient, ["jobs", spawnedFrom], jobList, wrapJobs, jobRecordId, entity_id, record);
          // Update embedded jobs_summary in-place for the parent issue
          const updated = updateJobSummaryInPaginatedIssues(queryClient, spawnedFrom, record);
          if (!updated) {
            // Parent issue not in any loaded page — fall back to invalidation
            queryClient.invalidateQueries({ queryKey: ["paginatedIssues"] });
          }
        } else {
          queryClient.invalidateQueries({ queryKey: ["jobs"] });
          // Job has no parent issue context — invalidate to be safe
          queryClient.invalidateQueries({ queryKey: ["paginatedIssues"] });
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
    queryClient.invalidateQueries({ queryKey: ["paginatedIssues"] });
    queryClient.invalidateQueries({ queryKey: ["jobs"] });
    queryClient.invalidateQueries({ queryKey: ["allJobs"] });
    queryClient.invalidateQueries({ queryKey: ["paginatedJobs"] });
    queryClient.invalidateQueries({ queryKey: ["patches"] });
    queryClient.invalidateQueries({ queryKey: ["documents"] });
    queryClient.invalidateQueries({ queryKey: ["paginatedDocuments"] });
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
