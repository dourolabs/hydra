import { useEffect, useRef, useState, useCallback } from "react";
import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import type {
  EntityEventData,
  IssueSummaryRecord,
  SessionSummaryRecord,
  ListIssuesResponse,
  ListSessionsResponse,
  ListRelationsResponse,
} from "@metis/api";

export type SSEConnectionState = "connecting" | "connected" | "disconnected";

const ENTITY_EVENT_TYPES = [
  "issue_created",
  "issue_updated",
  "issue_deleted",
  "patch_created",
  "patch_updated",
  "patch_deleted",
  "session_created",
  "session_updated",
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

const sessionList = (r: ListSessionsResponse) => r.sessions;
const wrapSessions = (items: SessionSummaryRecord[]): ListSessionsResponse => ({ sessions: items });
const sessionRecordId = (r: SessionSummaryRecord) => r.session_id;

// ---------------------------------------------------------------------------
// Tree cache mutation helpers — update usePageIssueTrees caches directly
// so that tree-derived UI (status boxes, active indicators, artifact lists)
// updates without re-fetching.
// ---------------------------------------------------------------------------

const wrapRels = (items: ListRelationsResponse["relations"]): ListRelationsResponse => ({
  relations: items,
});

/** Add a child-of relation to all matching relation caches (direct + transitive). */
function addChildOfRelation(
  qc: QueryClient,
  sourceId: string,
  targetId: string,
) {
  const rel = { source_id: sourceId, target_id: targetId, rel_type: "child-of" as const };
  // Update direct child-of caches
  qc.setQueriesData<ListRelationsResponse>(
    { queryKey: ["relations", "child-of"] },
    (old) => {
      if (!old) return old;
      // Avoid duplicates
      if (old.relations.some((r) => r.source_id === sourceId && r.target_id === targetId)) {
        return old;
      }
      return wrapRels([...old.relations, rel]);
    },
  );
}

/** Remove a child-of relation from all matching relation caches. */
function removeChildOfRelation(
  qc: QueryClient,
  sourceId: string,
) {
  qc.setQueriesData<ListRelationsResponse>(
    { queryKey: ["relations", "child-of"] },
    (old) => {
      if (!old) return old;
      const filtered = old.relations.filter((r) => r.source_id !== sourceId);
      if (filtered.length === old.relations.length) return old;
      return wrapRels(filtered);
    },
  );
}

/** Upsert an issue record into batch issue caches used by usePageIssueTrees. */
function upsertBatchIssue(qc: QueryClient, entityId: string, record: IssueSummaryRecord) {
  upsertInList(qc, ["issues", "batch"], issueList, wrapIssues, issueRecordId, entityId, record);
}

/** Remove an issue from batch issue caches. */
function removeBatchIssue(qc: QueryClient, entityId: string) {
  removeFromList(qc, ["issues", "batch"], issueList, wrapIssues, issueRecordId, entityId);
}

/** Upsert a session record into batch session caches used by usePageIssueTrees. */
function upsertBatchSession(qc: QueryClient, entityId: string, record: SessionSummaryRecord) {
  upsertInList(qc, ["sessions", "batch"], sessionList, wrapSessions, sessionRecordId, entityId, record);
}

// ---------------------------------------------------------------------------
// Targeted cache invalidation — used on resync and visibility change to
// refresh only the page-level and tree-level caches instead of all caches.
// ---------------------------------------------------------------------------

function invalidatePageAndTreeCaches(qc: QueryClient) {
  // Issue list caches (paginated dashboard)
  qc.invalidateQueries({ queryKey: ["issues"] });
  // Tree relationship caches
  qc.invalidateQueries({ queryKey: ["relations"] });
  // Batch issue/session caches used by usePageIssueTrees
  qc.invalidateQueries({ queryKey: ["issues", "batch"] });
  qc.invalidateQueries({ queryKey: ["sessions", "batch"] });
  qc.invalidateQueries({ queryKey: ["sessions"] });
  // Paginated document caches
  qc.invalidateQueries({ queryKey: ["paginatedDocuments"] });
  // Labels
  qc.invalidateQueries({ queryKey: ["labels"] });
}

/**
 * SSE hook that connects to the BFF /api/v1/events endpoint, listens for
 * entity mutation events, and updates React Query caches directly from the
 * entity data included in the event payload to avoid unnecessary HTTP
 * re-fetches.
 */
export function useSSE(): SSEConnectionState {
  const [state, setState] = useState<SSEConnectionState>("disconnected");
  const queryClient = useQueryClient();
  const retriesRef = useRef(0);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const esRef = useRef<EventSource | null>(null);

  /** Apply a direct cache update from SSE entity data. */
  const handleEntityEvent = useCallback(
    (eventType: string, data: EntityEventData) => {
      const { entity_type, entity_id, entity } = data;

      if (entity_type === "issue" || eventType.startsWith("issue_")) {
        if (eventType === "issue_deleted") {
          queryClient.removeQueries({ queryKey: ["issue", entity_id] });
          removeFromList(queryClient, ["issues"], issueList, wrapIssues, issueRecordId, entity_id);
          // Remove from tree caches
          removeBatchIssue(queryClient, entity_id);
          removeChildOfRelation(queryClient, entity_id);
        } else {
          const record = entity as unknown as IssueSummaryRecord;
          queryClient.invalidateQueries({ queryKey: ["issue", entity_id] });
          upsertInList(queryClient, ["issues"], issueList, wrapIssues, issueRecordId, entity_id, record);
          queryClient.invalidateQueries({ queryKey: ["issue", entity_id, "versions"] });
          // Directly update batch issue caches so child statuses recompute
          upsertBatchIssue(queryClient, entity_id, record);
          // Update child-of relation caches from the issue's dependencies
          if (eventType === "issue_created") {
            const deps = record.issue?.dependencies ?? [];
            for (const dep of deps) {
              if (dep.type === "child-of") {
                addChildOfRelation(queryClient, entity_id, dep.issue_id);
              }
            }
          }
        }
      } else if (entity_type === "session" || eventType.startsWith("session_")) {
        const record = entity as unknown as SessionSummaryRecord;
        const spawnedFrom = record.session?.spawned_from;

        queryClient.invalidateQueries({ queryKey: ["session", entity_id] });

        if (spawnedFrom) {
          upsertInList(queryClient, ["sessions", spawnedFrom], sessionList, wrapSessions, sessionRecordId, entity_id, record);
        } else {
          queryClient.invalidateQueries({ queryKey: ["sessions"] });
        }
        // Directly update batch session caches so hasActiveTask recomputes
        upsertBatchSession(queryClient, entity_id, record);
      } else if (entity_type === "patch" || eventType.startsWith("patch_")) {
        if (eventType === "patch_deleted") {
          queryClient.removeQueries({ queryKey: ["patch", entity_id] });
        }
        queryClient.invalidateQueries({ queryKey: ["patch", entity_id] });
        queryClient.invalidateQueries({ queryKey: ["patches"] });
      } else if (entity_type === "document" || eventType.startsWith("document_")) {
        if (eventType === "document_deleted") {
          queryClient.removeQueries({ queryKey: ["document", entity_id] });
        }
        queryClient.invalidateQueries({ queryKey: ["document", entity_id] });
        queryClient.invalidateQueries({ queryKey: ["paginatedDocuments"] });
      } else if (entity_type === "label" || eventType.startsWith("label_")) {
        queryClient.invalidateQueries({ queryKey: ["labels"] });
      }
    },
    [queryClient],
  );

  const connect = useCallback(() => {
    // Clean up previous connection
    if (esRef.current) {
      esRef.current.close();
      esRef.current = null;
    }

    setState("connecting");

    const es = new EventSource("/api/v1/events?types=issues,sessions,patches,documents,labels");
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

    // Connected event on initial connection — no action needed
    es.addEventListener("connected", () => {
      // Server confirmed connection with current seq. No action needed.
    });

    // Resync event — client has fallen behind, invalidate page and tree caches
    // only (not all caches globally) to trigger targeted refetches.
    es.addEventListener("resync", () => {
      invalidatePageAndTreeCaches(queryClient);
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
  }, [handleEntityEvent, queryClient]);

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
        invalidatePageAndTreeCaches(queryClient);
        connect();
      }
    };

    document.addEventListener("visibilitychange", handleVisibilityChange);
    return () => {
      document.removeEventListener("visibilitychange", handleVisibilityChange);
    };
  }, [connect, queryClient]);

  return state;
}
