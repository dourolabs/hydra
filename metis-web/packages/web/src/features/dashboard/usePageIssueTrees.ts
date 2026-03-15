import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import type {
  IssueSummaryRecord,
  SessionSummaryRecord,
} from "@metis/api";
import { apiClient } from "../../api/client";
import type { ChildStatus } from "./computeIssueProgress";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface IssueTreeData {
  childStatuses: ChildStatus[];
  isActive: boolean;
  patchIds: string[];
  documentIds: string[];
}

// ---------------------------------------------------------------------------
// Step 1: Fetch direct child relations for page issue IDs
// ---------------------------------------------------------------------------

function useChildRelations(pageIssueIds: string[]) {
  const targetIds = pageIssueIds.join(",");
  return useQuery({
    queryKey: ["relations", "child-of", targetIds],
    queryFn: () =>
      apiClient.listRelations({
        target_ids: targetIds,
        rel_type: "child-of",
      }),
    enabled: pageIssueIds.length > 0,
    staleTime: 30_000,
    select: (data) => data.relations,
  });
}

// ---------------------------------------------------------------------------
// Step 2: Fetch transitive descendants for issues that have children
// ---------------------------------------------------------------------------

function useTransitiveRelations(parentIdsWithChildren: string[]) {
  const targetIds = parentIdsWithChildren.join(",");
  return useQuery({
    queryKey: ["relations", "child-of", "transitive", targetIds],
    queryFn: () =>
      apiClient.listRelations({
        target_ids: targetIds,
        rel_type: "child-of",
        transitive: true,
      }),
    enabled: parentIdsWithChildren.length > 0,
    staleTime: 30_000,
    select: (data) => data.relations,
  });
}

// ---------------------------------------------------------------------------
// Step 3: Fetch child issue details
// ---------------------------------------------------------------------------

function useChildIssueDetails(childIds: string[]) {
  const ids = childIds.join(",");
  return useQuery({
    queryKey: ["issues", "batch", ids],
    queryFn: () => apiClient.listIssues({ ids }),
    enabled: childIds.length > 0,
    staleTime: 30_000,
    select: (data) => data.issues,
  });
}

// ---------------------------------------------------------------------------
// Step 4: Fetch sessions for all descendants
// ---------------------------------------------------------------------------

function useDescendantSessions(descendantIds: string[]) {
  const spawned_from_ids = descendantIds.join(",");
  return useQuery({
    queryKey: ["sessions", "batch", spawned_from_ids],
    queryFn: () =>
      apiClient.listSessions({ spawned_from_ids }),
    enabled: descendantIds.length > 0,
    staleTime: 30_000,
    select: (data) => data.sessions,
  });
}

// ---------------------------------------------------------------------------
// Step 5: Fetch artifact relations (patches, documents)
// ---------------------------------------------------------------------------

function usePatchRelations(pageIssueIds: string[], allDescendantIds: string[]) {
  const allIds = [...new Set([...pageIssueIds, ...allDescendantIds])];
  const objectIds = allIds.join(",");
  return useQuery({
    queryKey: ["relations", "has-patch", objectIds],
    queryFn: () =>
      apiClient.listRelations({
        target_ids: objectIds,
        rel_type: "has-patch",
      }),
    enabled: allIds.length > 0,
    staleTime: 30_000,
    select: (data) => data.relations,
  });
}

function useDocumentRelations(pageIssueIds: string[], allDescendantIds: string[]) {
  const allIds = [...new Set([...pageIssueIds, ...allDescendantIds])];
  const objectIds = allIds.join(",");
  return useQuery({
    queryKey: ["relations", "has-document", objectIds],
    queryFn: () =>
      apiClient.listRelations({
        target_ids: objectIds,
        rel_type: "has-document",
      }),
    enabled: allIds.length > 0,
    staleTime: 30_000,
    select: (data) => data.relations,
  });
}

// ---------------------------------------------------------------------------
// Helper: group sessions by spawned_from issue ID
// ---------------------------------------------------------------------------

function groupSessionsByIssue(
  sessions: SessionSummaryRecord[],
): Map<string, SessionSummaryRecord[]> {
  const map = new Map<string, SessionSummaryRecord[]>();
  for (const session of sessions) {
    const issueId = session.session.spawned_from;
    if (!issueId) continue;
    const list = map.get(issueId);
    if (list) {
      list.push(session);
    } else {
      map.set(issueId, [session]);
    }
  }
  return map;
}

// ---------------------------------------------------------------------------
// Helper: check if any descendant has active sessions
// ---------------------------------------------------------------------------

function hasActiveSession(
  issueId: string,
  childrenMap: Map<string, string[]>,
  sessionsByIssue: Map<string, SessionSummaryRecord[]>,
  cache: Map<string, boolean>,
): boolean {
  const cached = cache.get(issueId);
  if (cached !== undefined) return cached;

  const sessions = sessionsByIssue.get(issueId) ?? [];
  if (sessions.some((s) => s.session.status === "running" || s.session.status === "pending")) {
    cache.set(issueId, true);
    return true;
  }

  const children = childrenMap.get(issueId) ?? [];
  const result = children.some((childId) =>
    hasActiveSession(childId, childrenMap, sessionsByIssue, cache),
  );
  cache.set(issueId, result);
  return result;
}

// ---------------------------------------------------------------------------
// Main hook
// ---------------------------------------------------------------------------

export function usePageIssueTrees(
  pageIssues: IssueSummaryRecord[],
  username: string,
) {
  const pageIssueIds = useMemo(
    () => pageIssues.map((i) => i.issue_id),
    [pageIssues],
  );

  // Step 1: Get direct children
  const { data: directChildRelations, isLoading: childRelLoading } =
    useChildRelations(pageIssueIds);

  // Determine which page issues have children
  const parentsWithChildren = useMemo(() => {
    if (!directChildRelations) return [];
    const parents = new Set<string>();
    for (const rel of directChildRelations) {
      parents.add(rel.target_id);
    }
    return Array.from(parents);
  }, [directChildRelations]);

  // Direct child IDs (for fetching details)
  const directChildIds = useMemo(() => {
    if (!directChildRelations) return [];
    const ids = new Set<string>();
    for (const rel of directChildRelations) {
      ids.add(rel.source_id);
    }
    return Array.from(ids);
  }, [directChildRelations]);

  // Step 2: Get transitive descendants
  const { data: transitiveRelations, isLoading: transitiveLoading } =
    useTransitiveRelations(parentsWithChildren);

  // All descendant IDs (direct + transitive)
  const allDescendantIds = useMemo(() => {
    const ids = new Set(directChildIds);
    if (transitiveRelations) {
      for (const rel of transitiveRelations) {
        ids.add(rel.source_id);
      }
    }
    return Array.from(ids);
  }, [directChildIds, transitiveRelations]);

  // Build parent→children map from all relations
  const childrenMap = useMemo(() => {
    const map = new Map<string, string[]>();
    const allRelations = [
      ...(directChildRelations ?? []),
      ...(transitiveRelations ?? []),
    ];
    for (const rel of allRelations) {
      const children = map.get(rel.target_id) ?? [];
      if (!children.includes(rel.source_id)) {
        children.push(rel.source_id);
      }
      map.set(rel.target_id, children);
    }
    return map;
  }, [directChildRelations, transitiveRelations]);

  // Step 3: Fetch child issue details
  const { data: childIssues, isLoading: childIssuesLoading } =
    useChildIssueDetails(directChildIds);

  // Step 4: Fetch sessions for all descendants + page issues
  const allIssueIds = useMemo(
    () => [...new Set([...pageIssueIds, ...allDescendantIds])],
    [pageIssueIds, allDescendantIds],
  );
  const { data: descendantSessions, isLoading: sessionsLoading } =
    useDescendantSessions(allIssueIds);

  // Step 5: Fetch artifact relations
  const { data: patchRelations, isLoading: patchRelLoading } =
    usePatchRelations(pageIssueIds, allDescendantIds);
  const { data: documentRelations, isLoading: docRelLoading } =
    useDocumentRelations(pageIssueIds, allDescendantIds);

  // Group sessions
  const sessionsByIssue = useMemo(
    () => groupSessionsByIssue(descendantSessions ?? []),
    [descendantSessions],
  );

  // Build per-issue tree data
  const treeDataMap = useMemo(() => {
    const map = new Map<string, IssueTreeData>();
    if (!directChildRelations) return map;

    const childIssueMap = new Map<string, IssueSummaryRecord>();
    if (childIssues) {
      for (const issue of childIssues) {
        childIssueMap.set(issue.issue_id, issue);
      }
    }

    const activeCache = new Map<string, boolean>();

    for (const pageIssueId of pageIssueIds) {
      // Child statuses
      const childIds = childrenMap.get(pageIssueId) ?? [];
      const childStatuses: ChildStatus[] = [];
      for (const childId of childIds) {
        const child = childIssueMap.get(childId);
        if (!child) continue;
        childStatuses.push({
          id: childId,
          status: child.issue.status,
          hasActiveTask: hasActiveSession(
            childId,
            childrenMap,
            sessionsByIssue,
            activeCache,
          ),
          assignedToUser: !!(username && child.issue.assignee === username),
        });
      }

      // isActive for the page issue itself
      const isActive = hasActiveSession(
        pageIssueId,
        childrenMap,
        sessionsByIssue,
        activeCache,
      );

      // Collect descendants for this page issue
      const descendants = new Set<string>([pageIssueId]);
      const queue = [pageIssueId];
      while (queue.length > 0) {
        const current = queue.shift()!;
        for (const cid of childrenMap.get(current) ?? []) {
          if (!descendants.has(cid)) {
            descendants.add(cid);
            queue.push(cid);
          }
        }
      }

      // Patch IDs from has-patch relations
      const patchIds: string[] = [];
      if (patchRelations) {
        for (const rel of patchRelations) {
          if (descendants.has(rel.target_id)) {
            patchIds.push(rel.source_id);
          }
        }
      }

      // Document IDs from has-document relations
      const documentIds: string[] = [];
      if (documentRelations) {
        for (const rel of documentRelations) {
          if (descendants.has(rel.target_id)) {
            documentIds.push(rel.source_id);
          }
        }
      }

      map.set(pageIssueId, {
        childStatuses,
        isActive,
        patchIds,
        documentIds,
      });
    }

    return map;
  }, [
    pageIssueIds,
    directChildRelations,
    childrenMap,
    childIssues,
    sessionsByIssue,
    patchRelations,
    documentRelations,
    username,
  ]);

  // isActiveMap for compatibility
  const isActiveMap = useMemo(() => {
    const map = new Map<string, boolean>();
    for (const [id, data] of treeDataMap) {
      map.set(id, data.isActive);
    }
    return map;
  }, [treeDataMap]);

  // childStatusMap for compatibility
  const childStatusMap = useMemo(() => {
    const map = new Map<string, ChildStatus[]>();
    for (const [id, data] of treeDataMap) {
      if (data.childStatuses.length > 0) {
        map.set(id, data.childStatuses);
      }
    }
    return map;
  }, [treeDataMap]);

  const isLoading =
    childRelLoading || transitiveLoading || childIssuesLoading || sessionsLoading || patchRelLoading || docRelLoading;

  return {
    treeDataMap,
    isActiveMap,
    childStatusMap,
    sessionsByIssue,
    isLoading,
  };
}
