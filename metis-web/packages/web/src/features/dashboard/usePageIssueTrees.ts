import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import type {
  IssueSummaryRecord,
  RelationResponse,
  SessionSummaryRecord,
} from "@hydra/api";
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

/** The complete collected set of data for building issue trees. */
interface CollectedSet {
  pageIssueIds: string[];
  childRelations: RelationResponse[];
  issueMap: Map<string, IssueSummaryRecord>;
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  patchRelations: RelationResponse[];
  documentRelations: RelationResponse[];
  username: string;
}

// ---------------------------------------------------------------------------
// Step 1 (Seed): Page issues come from the caller
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Step 2 (Expand): Collect all transitive descendants via child-of relations
// ---------------------------------------------------------------------------

function useExpandedChildRelations(pageIssueIds: string[]) {
  const sortedIds = useMemo(
    () => [...pageIssueIds].sort(),
    [pageIssueIds],
  );
  const targetIds = sortedIds.join(",");
  return useQuery({
    queryKey: ["relations", "child-of", "transitive", ...sortedIds],
    queryFn: () =>
      apiClient.listRelations({
        target_ids: targetIds,
        rel_type: "child-of",
        transitive: true,
      }),
    enabled: pageIssueIds.length > 0,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
    select: (data) => data.relations,
  });
}

// ---------------------------------------------------------------------------
// Step 3 (Artifacts): Collect patches and documents for expanded set
// ---------------------------------------------------------------------------

function useArtifactRelations(
  allIssueIds: string[],
  relType: "has-patch" | "has-document",
) {
  const targetIds = allIssueIds.join(",");
  return useQuery({
    queryKey: ["relations", relType, targetIds],
    queryFn: () =>
      apiClient.listRelations({
        target_ids: targetIds,
        rel_type: relType,
      }),
    enabled: allIssueIds.length > 0,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
    select: (data) => data.relations,
  });
}

// ---------------------------------------------------------------------------
// Step 4 (Sessions): Collect sessions for expanded set
// ---------------------------------------------------------------------------

function useSessions(allIssueIds: string[]) {
  const spawned_from_ids = allIssueIds.join(",");
  return useQuery({
    queryKey: ["sessions", "batch", spawned_from_ids],
    queryFn: () =>
      apiClient.listSessions({ spawned_from_ids }),
    enabled: allIssueIds.length > 0,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
    select: (data) => data.sessions,
  });
}

// ---------------------------------------------------------------------------
// Step 5 (Summary records): Fetch issue summaries for all descendants
// ---------------------------------------------------------------------------

function useIssueSummaries(descendantIds: string[]) {
  const ids = descendantIds.join(",");
  return useQuery({
    queryKey: ["issues", "batch", ids],
    queryFn: () => apiClient.listIssues({ ids }),
    enabled: descendantIds.length > 0,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
    select: (data) => data.issues,
  });
}

// ---------------------------------------------------------------------------
// Step 6 (Build trees): Pure function that builds tree data from collected set
// ---------------------------------------------------------------------------

function buildIssueTrees(set: CollectedSet): {
  treeDataMap: Map<string, IssueTreeData>;
  isActiveMap: Map<string, boolean>;
  childStatusMap: Map<string, ChildStatus[]>;
} {
  const treeDataMap = new Map<string, IssueTreeData>();

  // Build parent→children map from child-of relations
  const childrenMap = new Map<string, string[]>();
  for (const rel of set.childRelations) {
    const children = childrenMap.get(rel.target_id) ?? [];
    if (!children.includes(rel.source_id)) {
      children.push(rel.source_id);
    }
    childrenMap.set(rel.target_id, children);
  }

  // Memoized active-session check
  const activeCache = new Map<string, boolean>();
  function isActive(issueId: string): boolean {
    const cached = activeCache.get(issueId);
    if (cached !== undefined) return cached;

    const sessions = set.sessionsByIssue.get(issueId) ?? [];
    if (
      sessions.some(
        (s) => s.session.status === "running" || s.session.status === "pending",
      )
    ) {
      activeCache.set(issueId, true);
      return true;
    }

    const children = childrenMap.get(issueId) ?? [];
    const result = children.some((childId) => isActive(childId));
    activeCache.set(issueId, result);
    return result;
  }

  for (const pageIssueId of set.pageIssueIds) {
    // Direct child statuses
    const directChildIds = childrenMap.get(pageIssueId) ?? [];
    const childStatuses: ChildStatus[] = [];
    for (const childId of directChildIds) {
      const child = set.issueMap.get(childId);
      if (!child) continue;
      childStatuses.push({
        id: childId,
        status: child.issue.status,
        hasActiveTask: isActive(childId),
        assignedToUser: !!(
          set.username && child.issue.assignee === set.username
        ),
      });
    }

    // Collect all descendants for artifact attribution
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

    // Patches linked to this issue's subtree
    const patchIds: string[] = [];
    for (const rel of set.patchRelations) {
      if (descendants.has(rel.target_id)) {
        patchIds.push(rel.source_id);
      }
    }

    // Documents linked to this issue's subtree
    const documentIds: string[] = [];
    for (const rel of set.documentRelations) {
      if (descendants.has(rel.target_id)) {
        documentIds.push(rel.source_id);
      }
    }

    treeDataMap.set(pageIssueId, {
      childStatuses,
      isActive: isActive(pageIssueId),
      patchIds,
      documentIds,
    });
  }

  // Derived maps for consumer convenience
  const isActiveMap = new Map<string, boolean>();
  const childStatusMap = new Map<string, ChildStatus[]>();
  for (const [id, data] of treeDataMap) {
    isActiveMap.set(id, data.isActive);
    if (data.childStatuses.length > 0) {
      childStatusMap.set(id, data.childStatuses);
    }
  }

  return { treeDataMap, isActiveMap, childStatusMap };
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
// Main hook
// ---------------------------------------------------------------------------

export function usePageIssueTrees(
  pageIssues: IssueSummaryRecord[],
  username: string,
) {
  // Step 1: Seed set — page issue IDs
  const pageIssueIds = useMemo(
    () => pageIssues.map((i) => i.issue_id),
    [pageIssues],
  );

  // Step 2: Expand — collect all transitive descendants
  const { data: childRelations } = useExpandedChildRelations(pageIssueIds);

  // Derive the full expanded issue set (page issues + all descendants)
  const allDescendantIds = useMemo(() => {
    if (!childRelations) return [];
    const ids = new Set<string>();
    for (const rel of childRelations) {
      ids.add(rel.source_id);
    }
    return Array.from(ids);
  }, [childRelations]);

  const allIssueIds = useMemo(
    () => [...new Set([...pageIssueIds, ...allDescendantIds])],
    [pageIssueIds, allDescendantIds],
  );

  // Step 3: Artifacts — collect patches and documents for expanded set
  const { data: patchRelations } = useArtifactRelations(allIssueIds, "has-patch");
  const { data: documentRelations } = useArtifactRelations(allIssueIds, "has-document");

  // Step 4: Sessions — collect sessions for expanded set
  const { data: sessions } = useSessions(allIssueIds);

  // Step 5: Summary records — fetch issue summaries for descendants
  const { data: descendantIssues } = useIssueSummaries(allDescendantIds);

  // Build lookup maps
  const issueMap = useMemo(() => {
    const map = new Map<string, IssueSummaryRecord>();
    for (const issue of pageIssues) {
      map.set(issue.issue_id, issue);
    }
    if (descendantIssues) {
      for (const issue of descendantIssues) {
        map.set(issue.issue_id, issue);
      }
    }
    return map;
  }, [pageIssues, descendantIssues]);

  const sessionsByIssue = useMemo(
    () => groupSessionsByIssue(sessions ?? []),
    [sessions],
  );

  // Step 6: Build trees — pure function of the collected set
  const { treeDataMap, isActiveMap, childStatusMap } = useMemo(
    () =>
      buildIssueTrees({
        pageIssueIds,
        childRelations: childRelations ?? [],
        issueMap,
        sessionsByIssue,
        patchRelations: patchRelations ?? [],
        documentRelations: documentRelations ?? [],
        username,
      }),
    [
      pageIssueIds,
      childRelations,
      issueMap,
      sessionsByIssue,
      patchRelations,
      documentRelations,
      username,
    ],
  );

  // Only show loading state on the very first fetch when no data (real or
  // placeholder) is available. With keepPreviousData, subsequent refetches
  // caused by changed query keys will still have placeholder data, so we
  // avoid flashing the shimmer animation.
  const isLoading =
    (pageIssueIds.length > 0 && childRelations === undefined) ||
    (allIssueIds.length > 0 && patchRelations === undefined) ||
    (allIssueIds.length > 0 && documentRelations === undefined) ||
    (allIssueIds.length > 0 && sessions === undefined) ||
    (allDescendantIds.length > 0 && descendantIssues === undefined);

  return {
    treeDataMap,
    isActiveMap,
    childStatusMap,
    sessionsByIssue,
    isLoading,
  };
}
