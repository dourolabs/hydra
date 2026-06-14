import { useEffect, useRef, useState, useCallback } from "react";
import { useQueryClient, type InfiniteData, type QueryClient } from "@tanstack/react-query";
import type {
  DocumentSummaryRecord,
  EntityEventData,
  IssueSummaryRecord,
  ListDocumentsResponse,
  ListIssuesResponse,
  ListSessionsResponse,
  SessionEvent,
  SessionLogEventData,
  SessionSummaryRecord,
  ConversationSummary,
} from "@hydra/api";
import { sessionLogRegistry } from "./sessionLogRegistry";

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
  "session_event_created",
  "session_state_updated",
  "document_created",
  "document_updated",
  "document_deleted",
  "label_created",
  "label_updated",
  "label_deleted",
  "conversation_created",
  "conversation_updated",
] as const;

const MAX_BACKOFF_MS = 15_000;
const BASE_BACKOFF_MS = 1_000;
// How long a connection must stay open before we treat the prior failure
// burst as resolved and reset the retry counter. Otherwise a single transient
// blip permanently parks the user near MAX_BACKOFF_MS.
const RETRIES_RESET_AFTER_OPEN_MS = 30_000;
const SESSION_IDS_RECONNECT_DEBOUNCE_MS = 200;
const BASE_EVENT_TYPES_QUERY = "types=issues,sessions,patches,documents,labels,conversations";

function buildEventsUrl(sessionIds: readonly string[]): string {
  if (sessionIds.length === 0) {
    return `/api/v1/events?${BASE_EVENT_TYPES_QUERY}`;
  }
  const ids = sessionIds.map((id) => encodeURIComponent(id)).join(",");
  return `/api/v1/events?${BASE_EVENT_TYPES_QUERY}&session_ids=${ids}`;
}

// ---------------------------------------------------------------------------
// Cache-update helpers — eliminate repeated version-guard & list-upsert logic.
//
// The handler patches three families of list-response caches:
//
//   1. Flat list-response caches: `["issues"]`, `["issues", "batch"]`,
//      `["sessions"]`, `["sessions", "batch"]`, `["sessions", spawned_from]`.
//      Shape: `{ <entity_plural>: TItem[] }`. Patched by `upsertInList` /
//      `removeFromList`.
//
//   2. Filtered paginated caches: `["paginatedIssues", filters, kind, …]`,
//      `["paginatedSessions", filters]`, `["paginatedDocuments", q]`,
//      `["sessions", "active", creator, limit]`. Shape varies per consumer
//      (InfiniteData, single response, array of pages, flat array). Patched
//      by `patchListCache` via per-shape wrappers below.
//
//   3. Detail / count / relation caches: invalidated, not patched, because a
//      single entity payload can't recompute them.
//
// Filtered caches are patched only after a *filter-match check*: an
// `issue_updated` whose new status no longer satisfies a `status=open`
// filter must be REMOVED from that cache, not added/updated. The match
// predicate returns `true` / `false` / `"unknown"` — `"unknown"` is the
// `q` (free-text search) escape hatch where we leave the cache alone and
// rely on reconnect/visibility-change to resync.
// ---------------------------------------------------------------------------

interface VersionedRecord {
  version: number | bigint;
}

type FilterMatch = boolean | "unknown";

/**
 * Version-guarded upsert into an array within a list-response cache entry.
 * Updates in place (with version guard) if the entity already exists, or
 * appends to cover newly-created entities.
 *
 * Accepts a `predicate` to scope which queries under `key` are patched.
 * Without it, `setQueriesData` prefix-matches into adjacent shapes (e.g.
 * `["issues", id, "comments"]`) whose updater would throw on access.
 */
function upsertInList<TResp, TItem extends VersionedRecord>(
  qc: QueryClient,
  key: readonly unknown[],
  getItems: (resp: TResp) => TItem[],
  wrapItems: (items: TItem[]) => TResp,
  getId: (item: TItem) => string,
  entityId: string,
  record: TItem,
  predicate?: (queryKey: readonly unknown[]) => boolean,
) {
  qc.setQueriesData<TResp>(
    {
      queryKey: key,
      ...(predicate ? { predicate: (q) => predicate(q.queryKey) } : {}),
    },
    (old) => {
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
    },
  );
}

/** Remove an entity from an array within a list-response cache entry. */
function removeFromList<TResp, TItem>(
  qc: QueryClient,
  key: readonly unknown[],
  getItems: (resp: TResp) => TItem[],
  wrapItems: (items: TItem[]) => TResp,
  getId: (item: TItem) => string,
  entityId: string,
  predicate?: (queryKey: readonly unknown[]) => boolean,
) {
  qc.setQueriesData<TResp>(
    {
      queryKey: key,
      ...(predicate ? { predicate: (q) => predicate(q.queryKey) } : {}),
    },
    (old) => {
      if (!old) return old;
      return wrapItems(getItems(old).filter((a) => getId(a) !== entityId));
    },
  );
}

/**
 * Shape-agnostic upsert/remove against a filtered list cache.
 *
 * Caches are normalised to a `TItem[][]` (a list of pages, even when there
 * is only one). The helper finds the record across pages and:
 *   - `isDelete`: removes it if present.
 *   - found + `match === false`: removes (record no longer satisfies filter).
 *   - found + match `true` | `"unknown"`: updates in place (version-guarded).
 *   - missing + `match === true`: prepends to page 0 (eventual-consistent
 *     ordering — the next refetch / sort pass will re-sort).
 *   - missing + match `false` | `"unknown"`: leaves the cache alone.
 *
 * `match === "unknown"` covers the free-text `q` filter: we can't replicate
 * the server's search logic client-side, so don't risk a wrong insertion or
 * removal. The cache will catch up via reconnect/visibility-change invalidate.
 */
function patchListCache<TCache, TItem extends VersionedRecord>(
  cache: TCache | undefined,
  toGroups: (c: TCache) => TItem[][],
  fromGroups: (c: TCache, groups: TItem[][]) => TCache,
  getId: (item: TItem) => string,
  entityId: string,
  record: TItem | null,
  match: FilterMatch,
  isDelete: boolean,
): TCache | undefined {
  if (cache === undefined) return cache;
  const groups = toGroups(cache);

  let groupIdx = -1;
  let itemIdx = -1;
  for (let i = 0; i < groups.length; i++) {
    const idx = groups[i].findIndex((it) => getId(it) === entityId);
    if (idx >= 0) {
      groupIdx = i;
      itemIdx = idx;
      break;
    }
  }

  if (isDelete) {
    if (groupIdx < 0) return cache;
    const next = groups.map((g, i) => (i === groupIdx ? g.filter((_, j) => j !== itemIdx) : g));
    return fromGroups(cache, next);
  }

  if (record === null) return cache;

  if (groupIdx >= 0) {
    const existing = groups[groupIdx][itemIdx];
    if (match === false) {
      const next = groups.map((g, i) => (i === groupIdx ? g.filter((_, j) => j !== itemIdx) : g));
      return fromGroups(cache, next);
    }
    if (existing.version > record.version) return cache;
    const next = groups.map((g, i) => {
      if (i !== groupIdx) return g;
      const arr = [...g];
      arr[itemIdx] = record;
      return arr;
    });
    return fromGroups(cache, next);
  }

  if (match !== true) return cache;
  if (groups.length === 0) return cache;
  const next = groups.map((g, i) => (i === 0 ? [record, ...g] : g));
  return fromGroups(cache, next);
}

// --- Filter-match predicates ----------------------------------------------
// Match is a best-effort client-side reproduction of the server-side filter
// SQL. The exact match (status, type, creator, project_id, ids, assignee,
// archived) is precise; free-text search (`q`) returns `"unknown"` so the
// helper falls back to "leave the cache alone for this query".

interface IssueListFilters {
  status?: string | null;
  type?: string | null;
  creator?: string | null;
  assignee?: string | null;
  labels?: string | null;
  q?: string | null;
  ids?: string | null;
  project_id?: string | null;
  include_archived?: boolean | null;
}

function issueMatchesFilters(filters: IssueListFilters, rec: IssueSummaryRecord): FilterMatch {
  if (filters.q) return "unknown";
  if (filters.labels) return "unknown";
  if (filters.assignee) return "unknown";
  if (filters.status) {
    const allowed = filters.status.split(",");
    if (!allowed.includes(rec.issue.status.key)) return false;
  }
  if (filters.type && filters.type !== rec.issue.type) return false;
  if (filters.creator && filters.creator !== rec.issue.creator) return false;
  if (filters.project_id && filters.project_id !== rec.issue.project_id) return false;
  if (filters.ids) {
    const ids = filters.ids.split(",");
    if (!ids.includes(rec.issue_id)) return false;
  }
  if (!filters.include_archived && rec.issue.archived) return false;
  return true;
}

interface SessionListFilters {
  status?: string | null;
  creator?: string | null;
  spawned_from_ids?: string | null;
  conversation_id?: string | null;
  q?: string | null;
}

function sessionMatchesFilters(
  filters: SessionListFilters,
  rec: SessionSummaryRecord,
): FilterMatch {
  if (filters.q) return "unknown";
  if (filters.status) {
    const allowed = filters.status.split(",");
    if (!allowed.includes(rec.session.status)) return false;
  }
  if (filters.creator && filters.creator !== rec.session.creator) return false;
  if (filters.spawned_from_ids) {
    const ids = filters.spawned_from_ids.split(",");
    if (!rec.session.spawned_from || !ids.includes(rec.session.spawned_from)) {
      return false;
    }
  }
  if (filters.conversation_id && filters.conversation_id !== rec.session.conversation_id) {
    return false;
  }
  return true;
}

const ACTIVE_SESSION_STATUSES = new Set(["created", "pending", "running"]);

function activeSessionMatches(creator: string | null, rec: SessionSummaryRecord): FilterMatch {
  if (!ACTIVE_SESSION_STATUSES.has(rec.session.status)) return false;
  if (creator && rec.session.creator !== creator) return false;
  return true;
}

// Entity-specific accessors for the list-response shapes
const issueList = (r: ListIssuesResponse) => r.issues;
const wrapIssues = (items: IssueSummaryRecord[]): ListIssuesResponse => ({ issues: items });
const issueRecordId = (r: IssueSummaryRecord) => r.issue_id;

const sessionList = (r: ListSessionsResponse) => r.sessions;
const wrapSessions = (items: SessionSummaryRecord[]): ListSessionsResponse => ({ sessions: items });
const sessionRecordId = (r: SessionSummaryRecord) => r.session_id;

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
  upsertInList(
    qc,
    ["sessions", "batch"],
    sessionList,
    wrapSessions,
    sessionRecordId,
    entityId,
    record,
  );
}

// --- Paginated-cache patchers --------------------------------------------
// Each wrapper targets one cache shape under a `[<entity>, …]` prefix and
// delegates to `patchListCache` for the actual upsert / remove / update.
//
// React Query v5's `setQueriesData` updater type only receives the cached
// data, not the query — but filter matching needs `query.queryKey[1]`. So
// these helpers iterate `getQueriesData` and call `setQueryData` per key.

/**
 * `usePaginatedIssues` table view: `["paginatedIssues", filters, "sort", sort]`
 * → `InfiniteData<ListIssuesResponse>`.
 */
function patchPaginatedIssuesInfinite(
  qc: QueryClient,
  entityId: string,
  record: IssueSummaryRecord | null,
  isDelete: boolean,
) {
  const matches = qc.getQueriesData<InfiniteData<ListIssuesResponse, string | undefined>>({
    queryKey: ["paginatedIssues"],
    predicate: (q) => q.queryKey[2] === "sort",
  });
  for (const [key] of matches) {
    qc.setQueryData<InfiniteData<ListIssuesResponse, string | undefined>>(key, (old) => {
      const filters = (key[1] as IssueListFilters) ?? {};
      const match: FilterMatch = record ? issueMatchesFilters(filters, record) : true;
      return patchListCache(
        old,
        (c) => c.pages.map((p) => p.issues),
        (c, groups) => ({
          ...c,
          pages: c.pages.map((p, i) => ({ ...p, issues: groups[i] })),
        }),
        issueRecordId,
        entityId,
        record,
        match,
        isDelete,
      );
    });
  }
}

/**
 * `useBoardIssuesByProject` bulk query: `["paginatedIssues", filters,
 * "board-bulk", sort]` → single `ListIssuesResponse`. Ordering is grouped
 * client-side by `(project_id, status.key)`; an inserted record lands at the
 * top of the bucket and the next refetch re-sorts.
 */
function patchPaginatedIssuesBoardBulk(
  qc: QueryClient,
  entityId: string,
  record: IssueSummaryRecord | null,
  isDelete: boolean,
) {
  const matches = qc.getQueriesData<ListIssuesResponse>({
    queryKey: ["paginatedIssues"],
    predicate: (q) => q.queryKey[2] === "board-bulk",
  });
  for (const [key] of matches) {
    qc.setQueryData<ListIssuesResponse>(key, (old) => {
      const filters = (key[1] as IssueListFilters) ?? {};
      const match: FilterMatch = record ? issueMatchesFilters(filters, record) : true;
      return patchListCache(
        old,
        (c) => [c.issues],
        (c, groups) => ({ ...c, issues: groups[0] }),
        issueRecordId,
        entityId,
        record,
        match,
        isDelete,
      );
    });
  }
}

/**
 * `useBoardIssuesByProject` per-cell expanded: `["paginatedIssues", filters,
 * "depth", n]` → `ListIssuesResponse[]` (array of pages).
 */
function patchPaginatedIssuesBoardDepth(
  qc: QueryClient,
  entityId: string,
  record: IssueSummaryRecord | null,
  isDelete: boolean,
) {
  const matches = qc.getQueriesData<ListIssuesResponse[]>({
    queryKey: ["paginatedIssues"],
    predicate: (q) => q.queryKey[2] === "depth",
  });
  for (const [key] of matches) {
    qc.setQueryData<ListIssuesResponse[]>(key, (old) => {
      const filters = (key[1] as IssueListFilters) ?? {};
      const match: FilterMatch = record ? issueMatchesFilters(filters, record) : true;
      return patchListCache(
        old,
        (pages) => pages.map((p) => p.issues),
        (pages, groups) => pages.map((p, i) => ({ ...p, issues: groups[i] })),
        issueRecordId,
        entityId,
        record,
        match,
        isDelete,
      );
    });
  }
}

/** Convenience: patch all three `["paginatedIssues", …]` shapes. */
function patchAllPaginatedIssues(
  qc: QueryClient,
  entityId: string,
  record: IssueSummaryRecord | null,
  isDelete: boolean,
) {
  patchPaginatedIssuesInfinite(qc, entityId, record, isDelete);
  patchPaginatedIssuesBoardBulk(qc, entityId, record, isDelete);
  patchPaginatedIssuesBoardDepth(qc, entityId, record, isDelete);
}

/**
 * `usePaginatedSessions`: `["paginatedSessions", filters]` →
 * `InfiniteData<ListSessionsResponse>`.
 */
function patchPaginatedSessions(
  qc: QueryClient,
  entityId: string,
  record: SessionSummaryRecord | null,
  isDelete: boolean,
) {
  const matches = qc.getQueriesData<InfiniteData<ListSessionsResponse, string | undefined>>({
    queryKey: ["paginatedSessions"],
  });
  for (const [key] of matches) {
    qc.setQueryData<InfiniteData<ListSessionsResponse, string | undefined>>(key, (old) => {
      const filters = (key[1] as SessionListFilters) ?? {};
      const match: FilterMatch = record ? sessionMatchesFilters(filters, record) : true;
      return patchListCache(
        old,
        (c) => c.pages.map((p) => p.sessions),
        (c, groups) => ({
          ...c,
          pages: c.pages.map((p, i) => ({ ...p, sessions: groups[i] })),
        }),
        sessionRecordId,
        entityId,
        record,
        match,
        isDelete,
      );
    });
  }
}

/**
 * `useActiveSessions` sidebar: `["sessions", "active", creator, limit]` →
 * flat `SessionSummaryRecord[]`. Same filter-match semantics: drop a session
 * that no longer matches `status ∈ {created, pending, running}` after an
 * update. Sized via the server-side `limit`; client-side we don't truncate.
 */
function patchActiveSessions(
  qc: QueryClient,
  entityId: string,
  record: SessionSummaryRecord | null,
  isDelete: boolean,
) {
  const matches = qc.getQueriesData<SessionSummaryRecord[]>({
    queryKey: ["sessions", "active"],
    predicate: (q) => q.queryKey.length >= 3,
  });
  for (const [key] of matches) {
    qc.setQueryData<SessionSummaryRecord[]>(key, (old) => {
      const creator = (key[2] as string | null) ?? null;
      const match: FilterMatch = record ? activeSessionMatches(creator, record) : true;
      return patchListCache(
        old,
        (arr) => [arr],
        (_arr, groups) => groups[0],
        sessionRecordId,
        entityId,
        record,
        match,
        isDelete,
      );
    });
  }
}

const documentRecordId = (r: DocumentSummaryRecord) => r.document_id;

/**
 * `usePaginatedDocuments`: `["paginatedDocuments", q]` →
 * `InfiniteData<ListDocumentsResponse>`. The filter is a single free-text
 * query string; if non-empty we can't reproduce the server's search, so we
 * still update in place when the record is already cached but never insert
 * or remove based on the search.
 */
function patchPaginatedDocuments(
  qc: QueryClient,
  entityId: string,
  record: DocumentSummaryRecord | null,
  isDelete: boolean,
) {
  const matches = qc.getQueriesData<InfiniteData<ListDocumentsResponse, string | undefined>>({
    queryKey: ["paginatedDocuments"],
  });
  for (const [key] of matches) {
    qc.setQueryData<InfiniteData<ListDocumentsResponse, string | undefined>>(key, (old) => {
      const q = (key[1] as string | null) ?? "";
      const match: FilterMatch = q ? "unknown" : true;
      return patchListCache(
        old,
        (c) => c.pages.map((p) => p.documents),
        (c, groups) => ({
          ...c,
          pages: c.pages.map((p, i) => ({ ...p, documents: groups[i] })),
        }),
        documentRecordId,
        entityId,
        record,
        match,
        isDelete,
      );
    });
  }
}

const conversationRecordId = (r: ConversationSummary) => r.conversation_id;

/**
 * Upsert a conversation summary into the conversations list caches. Two
 * cache shapes live under the `["conversations"]` prefix:
 * - `useConversations` (`["conversations", query]`) caches a flat
 *   `ConversationSummary[]`.
 * - `useActiveConversationsByIssue` (`["conversations", "batch", ids]`)
 *   caches the wrapped `ListConversationsResponse`. The batch shape needs
 *   to be patched too, or the board view's chat affordance won't appear
 *   until the user refreshes the page.
 */
function upsertBatchConversation(qc: QueryClient, entityId: string, record: ConversationSummary) {
  qc.setQueriesData<ConversationSummary[]>(
    { queryKey: ["conversations"], predicate: (q) => q.queryKey[1] !== "batch" },
    (old) => {
      if (!old) return old;
      const idx = old.findIndex((c) => conversationRecordId(c) === entityId);
      if (idx >= 0) {
        const updated = [...old];
        updated[idx] = record;
        return updated;
      }
      return [...old, record];
    },
  );
  qc.setQueriesData<{ conversations: ConversationSummary[]; next_cursor?: string | null }>(
    { queryKey: ["conversations", "batch"] },
    (old) => {
      if (!old) return old;
      const idx = old.conversations.findIndex((c) => conversationRecordId(c) === entityId);
      let next: ConversationSummary[];
      if (idx >= 0) {
        next = [...old.conversations];
        next[idx] = record;
      } else {
        next = [...old.conversations, record];
      }
      return { ...old, conversations: next };
    },
  );
}

// ---------------------------------------------------------------------------
// Targeted cache invalidation — used on resync and visibility change to
// refresh only the page-level and tree-level caches instead of all caches.
// ---------------------------------------------------------------------------

function invalidatePageAndTreeCaches(qc: QueryClient) {
  // Issue list caches (paginated dashboard)
  qc.invalidateQueries({ queryKey: ["issues"] });
  // Paginated issue list and badge counts
  qc.invalidateQueries({ queryKey: ["paginatedIssues"] });
  qc.invalidateQueries({ queryKey: ["issueCount"] });
  // Tree relationship caches
  qc.invalidateQueries({ queryKey: ["relations"] });
  // Batch issue/session caches used by usePageIssueTrees
  qc.invalidateQueries({ queryKey: ["issues", "batch"] });
  qc.invalidateQueries({ queryKey: ["sessions", "batch"] });
  qc.invalidateQueries({ queryKey: ["sessions"] });
  // /sessions paginated list and eyebrow count (separate root keys)
  qc.invalidateQueries({ queryKey: ["paginatedSessions"] });
  qc.invalidateQueries({ queryKey: ["sessionCount"] });
  // Patch list caches
  qc.invalidateQueries({ queryKey: ["patches"] });
  // Paginated document caches
  qc.invalidateQueries({ queryKey: ["paginatedDocuments"] });
  // Labels
  qc.invalidateQueries({ queryKey: ["labels"] });
  // Conversations
  qc.invalidateQueries({ queryKey: ["conversations"] });
  // Chat page Related tab caches
  qc.invalidateQueries({ queryKey: ["chatRelated"] });
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
  const retriesResetTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const esRef = useRef<EventSource | null>(null);
  const lastEventIdRef = useRef<string | null>(null);
  const invalidateTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const connectingRef = useRef(false);
  const sessionIdsReconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const currentSessionIdsKeyRef = useRef<string>("");

  const debouncedInvalidate = useCallback(() => {
    if (invalidateTimerRef.current) {
      clearTimeout(invalidateTimerRef.current);
    }
    invalidateTimerRef.current = setTimeout(() => {
      invalidateTimerRef.current = null;
      invalidatePageAndTreeCaches(queryClient);
    }, 100);
  }, [queryClient]);

  /** Apply a direct cache update from SSE entity data. */
  const handleEntityEvent = useCallback(
    (eventType: string, data: EntityEventData) => {
      const { entity_type, entity_id, entity } = data;

      // `session_state` events deliberately carry no entity payload (the
      // state blob is fetched separately via `get_session_state`); every
      // other entity type requires `entity` to be present.
      if (entity == null && entity_type !== "session_state") return;

      if (entity_type === "issue" || eventType.startsWith("issue_")) {
        if (eventType === "issue_deleted") {
          queryClient.removeQueries({ queryKey: ["issue", entity_id] });
          // The `["issues"]` prefix also covers the `["issues", id, "comments"]`
          // infinite cache whose shape lacks a `.issues` field — the predicate
          // restricts the patch to the bare `["issues"]` flat list.
          removeFromList(
            queryClient,
            ["issues"],
            issueList,
            wrapIssues,
            issueRecordId,
            entity_id,
            (qk) => qk.length === 1,
          );
          // Remove from tree caches
          removeBatchIssue(queryClient, entity_id);
          // Remove from all `["paginatedIssues", …]` cache shapes.
          patchAllPaginatedIssues(queryClient, entity_id, null, true);
          // Invalidate transitive child-of relation caches so trees recompute
          queryClient.invalidateQueries({ queryKey: ["relations", "child-of"] });
        } else {
          const record = entity as unknown as IssueSummaryRecord;
          queryClient.invalidateQueries({ queryKey: ["issue", entity_id] });
          // Same P9 predicate: skip the comments infinite cache that shares
          // the `["issues"]` prefix.
          upsertInList(
            queryClient,
            ["issues"],
            issueList,
            wrapIssues,
            issueRecordId,
            entity_id,
            record,
            (qk) => qk.length === 1,
          );
          queryClient.invalidateQueries({ queryKey: ["issue", entity_id, "versions"] });
          // Directly update batch issue caches so child statuses recompute
          upsertBatchIssue(queryClient, entity_id, record);
          // Patch all three `["paginatedIssues", …]` cache shapes in place
          // (table-view InfiniteData, board bulk-bucketed, board per-cell
          // expanded). Filter-aware: an issue whose new status no longer
          // satisfies a `status=open` cache is REMOVED from that cache.
          patchAllPaginatedIssues(queryClient, entity_id, record, false);
          // Invalidate transitive child-of relation caches when a new issue
          // is created (its child-of dependencies change the tree structure)
          if (eventType === "issue_created") {
            queryClient.invalidateQueries({ queryKey: ["relations", "child-of"] });
          }
        }
        // Badge count can't be patched from a single entity payload (filter
        // membership of OTHER issues may also have shifted).
        queryClient.invalidateQueries({ queryKey: ["issueCount"] });
        // Chat Related tab: prefix-match invalidates refers-to, referencedIssues,
        // referencedPatches, referencedDocuments (new issues may add relations
        // via link_conversation_to_artifacts; updates flow status/title through).
        queryClient.invalidateQueries({ queryKey: ["chatRelated"] });
      } else if (entity_type === "session_event") {
        // Live-tail append for the SessionEvent read path consumed by
        // `useChatTranscript`. The SSE payload carries the full SessionEvent,
        // so append it directly into the per-session events cache instead of
        // invalidating-then-refetching. `entity_id` is the session_id.
        const evt = entity as unknown as SessionEvent;
        queryClient.setQueryData<SessionEvent[]>(["sessionEvents", entity_id], (old) =>
          old ? [...old, evt] : old,
        );
      } else if (entity_type === "session_state") {
        // SessionState SSE notifications carry no payload; consumers must
        // refetch the state blob themselves. No current React Query consumer
        // reads SessionState (the chat transcript doesn't), so for now we
        // only invalidate the conventional per-session state key so that
        // future hooks pick the update up automatically.
        queryClient.invalidateQueries({ queryKey: ["sessionState", entity_id] });
      } else if (
        entity_type === "session" ||
        eventType === "session_created" ||
        eventType === "session_updated"
      ) {
        // Real server emits `entity_type = "session"` (singular). The mock
        // server emits the collection name `"sessions"` (plural), so fall
        // back to the event type — same shape used by the patch / document
        // / label / conversation branches below. The session_event and
        // session_state branches above intercept their own event types
        // first, so this fallback only catches `session_created` /
        // `session_updated`.
        const record = entity as unknown as SessionSummaryRecord;
        const spawnedFrom = record.session?.spawned_from;

        queryClient.invalidateQueries({ queryKey: ["session", entity_id] });
        // Per-session `proxy_targets` is keyed on session_id. A worker
        // advertising a port mid-conversation emits `session_updated`; the
        // chat page's ProxyTab won't render the new target until this cache
        // is refreshed (the hook's `refetchOnMount: "always"` only triggers
        // on navigate-back, which would otherwise be the only refresh path).
        queryClient.invalidateQueries({ queryKey: ["proxyTargets", entity_id] });

        if (spawnedFrom) {
          upsertInList(
            queryClient,
            ["sessions", spawnedFrom],
            sessionList,
            wrapSessions,
            sessionRecordId,
            entity_id,
            record,
          );
        }
        // Sidebar active-sessions list: filter-aware upsert (status flipping
        // out of {created, pending, running} REMOVES the session). Sidebar
        // badge count (`activeCount`) can't be derived from one record so
        // stays an invalidation.
        patchActiveSessions(queryClient, entity_id, record, false);
        queryClient.invalidateQueries({ queryKey: ["sessions", "activeCount"] });
        // /sessions page pagination patched in place across every filtered
        // InfiniteData cache. Eyebrow total stays an invalidation (a single
        // record can't recompute aggregate counts under arbitrary filters).
        patchPaginatedSessions(queryClient, entity_id, record, false);
        queryClient.invalidateQueries({ queryKey: ["sessionCount"] });
        // Directly update batch session caches so hasActiveTask recomputes
        upsertBatchSession(queryClient, entity_id, record);
      } else if (entity_type === "patch" || eventType.startsWith("patch_")) {
        if (eventType === "patch_deleted") {
          queryClient.removeQueries({ queryKey: ["patch", entity_id] });
        }
        queryClient.invalidateQueries({ queryKey: ["patch", entity_id] });
        queryClient.invalidateQueries({ queryKey: ["patches"] });
        // Invalidate has-patch relation caches so dashboard artifact lists refresh
        queryClient.invalidateQueries({ queryKey: ["relations", "has-patch"] });
        // Chat Related tab: prefix-match invalidates all referenced-artifact caches
        queryClient.invalidateQueries({ queryKey: ["chatRelated"] });
      } else if (entity_type === "document" || eventType.startsWith("document_")) {
        if (eventType === "document_deleted") {
          queryClient.removeQueries({ queryKey: ["document", entity_id] });
          patchPaginatedDocuments(queryClient, entity_id, null, true);
        } else {
          const record = entity as unknown as DocumentSummaryRecord;
          // Patch paginated document caches in place. Free-text `q` filter
          // is the "unknown" escape hatch: existing entries are updated in
          // place, but no insertion or removal is attempted against an
          // active search.
          patchPaginatedDocuments(queryClient, entity_id, record, false);
        }
        queryClient.invalidateQueries({ queryKey: ["document", entity_id] });
        // DocumentsPage tree: a doc mutation can add/remove path segments;
        // the tree key has a different shape so it stays an invalidation.
        queryClient.invalidateQueries({ queryKey: ["documentPathsBatch"] });
        // Invalidate has-document relation caches so dashboard artifact lists refresh
        queryClient.invalidateQueries({ queryKey: ["relations", "has-document"] });
        // Chat Related tab: prefix-match invalidates all referenced-artifact caches
        queryClient.invalidateQueries({ queryKey: ["chatRelated"] });
      } else if (entity_type === "label" || eventType.startsWith("label_")) {
        queryClient.invalidateQueries({ queryKey: ["labels"] });
      } else if (entity_type === "conversation" || eventType.startsWith("conversation_")) {
        // conversation_created or conversation_updated
        const record = entity as unknown as ConversationSummary;
        queryClient.invalidateQueries({ queryKey: ["conversation", entity_id] });
        upsertBatchConversation(queryClient, entity_id, record);
      }
    },
    [queryClient],
  );

  const connect = useCallback(() => {
    // Guard against duplicate connections
    if (esRef.current?.readyState === EventSource.OPEN) return;
    if (connectingRef.current) return;
    connectingRef.current = true;

    // Clean up previous connection
    if (esRef.current) {
      esRef.current.close();
      esRef.current = null;
    }

    setState("connecting");

    const sessionIds = sessionLogRegistry.sessionIds();
    currentSessionIdsKeyRef.current = sessionIds.join(",");
    const es = new EventSource(buildEventsUrl(sessionIds));
    esRef.current = es;

    es.onopen = () => {
      setState("connected");
      connectingRef.current = false;

      // Reset the retry counter only after the connection has been stable for
      // a sustained period; an immediate reset would let a flapping connection
      // hammer the server with short-delay reconnects.
      if (retriesResetTimerRef.current) {
        clearTimeout(retriesResetTimerRef.current);
      }
      retriesResetTimerRef.current = setTimeout(() => {
        retriesResetTimerRef.current = null;
        retriesRef.current = 0;
      }, RETRIES_RESET_AFTER_OPEN_MS);

      // If this is a reconnection (we previously received events), invalidate
      // caches to cover any events missed during the disconnect window.
      if (lastEventIdRef.current !== null) {
        debouncedInvalidate();
      }
    };

    // Entity mutation events
    for (const eventType of ENTITY_EVENT_TYPES) {
      es.addEventListener(eventType, (e: MessageEvent) => {
        if (e.lastEventId) {
          lastEventIdRef.current = e.lastEventId;
        }
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
      debouncedInvalidate();
    });

    // Heartbeat — keep-alive, no action needed
    es.addEventListener("heartbeat", () => {
      // No-op: confirms connection is alive
    });

    // Multiplexed session log chunks for any session IDs included in the
    // EventSource URL via the `session_ids` filter — route each chunk to its
    // SessionLogViewer subscriber(s) via the registry.
    es.addEventListener("session_log", (e: MessageEvent) => {
      if (e.lastEventId) {
        lastEventIdRef.current = e.lastEventId;
      }
      try {
        const data: SessionLogEventData = JSON.parse(e.data);
        sessionLogRegistry.dispatch(data.session_id, data.chunk);
      } catch {
        // Ignore malformed payloads
      }
    });

    es.onerror = () => {
      es.close();
      esRef.current = null;
      connectingRef.current = false;
      // Cancel the pending stable-open reset; the connection failed before
      // staying open long enough to count as recovered.
      if (retriesResetTimerRef.current) {
        clearTimeout(retriesResetTimerRef.current);
        retriesResetTimerRef.current = null;
      }
      setState("disconnected");

      // Half-jittered exponential backoff: each client picks a delay in
      // [ceiling/2, ceiling] so synchronized reconnect storms (e.g., after a
      // BFF restart) spread out instead of stampeding.
      const ceiling = Math.min(BASE_BACKOFF_MS * 2 ** retriesRef.current, MAX_BACKOFF_MS);
      const delay = ceiling * (0.5 + Math.random() * 0.5);
      retriesRef.current += 1;
      timerRef.current = setTimeout(connect, delay);
    };
  }, [debouncedInvalidate, handleEntityEvent, queryClient]);

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
      if (retriesResetTimerRef.current) {
        clearTimeout(retriesResetTimerRef.current);
        retriesResetTimerRef.current = null;
      }
      if (invalidateTimerRef.current) {
        clearTimeout(invalidateTimerRef.current);
        invalidateTimerRef.current = null;
      }
      if (sessionIdsReconnectTimerRef.current) {
        clearTimeout(sessionIdsReconnectTimerRef.current);
        sessionIdsReconnectTimerRef.current = null;
      }
    };
  }, [connect]);

  // EventSource URLs can't be changed after construction — when the registered
  // SessionLogViewer set changes, close and reopen the EventSource with the
  // updated `session_ids` filter. Debounce so rapid mount/unmount churn (e.g.,
  // navigating between session pages) coalesces into a single reconnect.
  useEffect(() => {
    const unsubscribe = sessionLogRegistry.onChange(() => {
      const nextKey = sessionLogRegistry.sessionIds().join(",");
      if (nextKey === currentSessionIdsKeyRef.current) {
        return;
      }
      if (sessionIdsReconnectTimerRef.current) {
        clearTimeout(sessionIdsReconnectTimerRef.current);
      }
      sessionIdsReconnectTimerRef.current = setTimeout(() => {
        sessionIdsReconnectTimerRef.current = null;
        if (sessionLogRegistry.sessionIds().join(",") === currentSessionIdsKeyRef.current) {
          return;
        }
        if (esRef.current) {
          esRef.current.close();
          esRef.current = null;
        }
        connectingRef.current = false;
        retriesRef.current = 0;
        if (timerRef.current) {
          clearTimeout(timerRef.current);
          timerRef.current = null;
        }
        connect();
      }, SESSION_IDS_RECONNECT_DEBOUNCE_MS);
    });

    return unsubscribe;
  }, [connect]);

  // Reconnect and refresh caches when the page becomes visible again or the
  // network comes back online (e.g., after mobile suspend or tab switch).
  //
  // Both transitions bypass the readyState guard inside `connect()` by force-
  // closing the existing EventSource first. After a suspend the underlying
  // TCP socket is often half-open: `readyState` still reads `OPEN` but no
  // data is flowing, and `onerror` won't fire until the OS hits its keepalive
  // timeout (Linux default tcp_keepalive_time = 7200s). Without this force-
  // close, `connect()` short-circuits and the user waits hours for recovery.
  useEffect(() => {
    const forceReconnect = () => {
      if (esRef.current) {
        esRef.current.close();
        esRef.current = null;
      }
      // The pending exponential-backoff timer (if any) targets the old
      // connection's failure; cancel it so the immediate connect() wins.
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
      if (retriesResetTimerRef.current) {
        clearTimeout(retriesResetTimerRef.current);
        retriesResetTimerRef.current = null;
      }
      retriesRef.current = 0;
      connectingRef.current = false;
      debouncedInvalidate();
      connect();
    };

    const handleVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        forceReconnect();
      }
    };

    const handleOnline = () => {
      forceReconnect();
    };

    document.addEventListener("visibilitychange", handleVisibilityChange);
    window.addEventListener("online", handleOnline);
    return () => {
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      window.removeEventListener("online", handleOnline);
    };
  }, [connect, debouncedInvalidate]);

  return state;
}
