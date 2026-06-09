import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import type { IssueSummaryRecord, SessionSummaryRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import type {
  IssueNeighborhood,
  NeighborStatus,
} from "../issues/flowPill";

// ---------------------------------------------------------------------------
// Step 1 (Seed): Page issues come from the caller
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Step 2 (Children): Direct children only — one hop along child-of. The
// FlowPill operates on the immediate neighborhood, so there is no transitive
// traversal here.
// ---------------------------------------------------------------------------

function useChildRelations(pageIssueIds: string[]) {
  const sortedIds = useMemo(() => [...pageIssueIds].sort(), [pageIssueIds]);
  const targetIds = sortedIds.join(",");
  return useQuery({
    queryKey: ["relations", "child-of", "direct", ...sortedIds],
    queryFn: () =>
      apiClient.listRelations({
        target_ids: targetIds,
        rel_type: "child-of",
      }),
    enabled: pageIssueIds.length > 0,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
    select: (data) => data.relations,
  });
}

// ---------------------------------------------------------------------------
// Step 3 (Sessions): Fetch sessions spawned from the page issues so callers
// can render runtime/duration. Sessions for descendants aren't needed under
// the local-neighborhood model.
// ---------------------------------------------------------------------------

function useSessions(pageIssueIds: string[]) {
  const spawned_from_ids = pageIssueIds.join(",");
  return useQuery({
    queryKey: ["sessions", "batch", spawned_from_ids],
    queryFn: () => apiClient.listSessions({ spawned_from_ids }),
    enabled: pageIssueIds.length > 0,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
    select: (data) => data.sessions,
  });
}

// ---------------------------------------------------------------------------
// Step 4 (Neighbor summaries): Fetch IssueSummary for every direct neighbor
// (children + blockers) not already in the page set.
// ---------------------------------------------------------------------------

function useIssueSummaries(neighborIds: string[]) {
  const ids = neighborIds.join(",");
  return useQuery({
    queryKey: ["issues", "batch", ids],
    queryFn: () => apiClient.listIssues({ ids }),
    enabled: neighborIds.length > 0,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
    select: (data) => data.issues,
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
// Main hook
// ---------------------------------------------------------------------------

export function usePageIssueTrees(pageIssues: IssueSummaryRecord[]) {
  const pageIssueIds = useMemo(
    () => pageIssues.map((i) => i.issue_id),
    [pageIssues],
  );

  const { data: childRelations } = useChildRelations(pageIssueIds);

  // Direct blocker IDs are baked into each issue's dependency list.
  const directBlockerIdsByIssue = useMemo(() => {
    const map = new Map<string, string[]>();
    for (const issue of pageIssues) {
      const blockerIds = issue.issue.dependencies
        .filter((d) => d.type === "blocked-on")
        .map((d) => d.issue_id);
      if (blockerIds.length > 0) map.set(issue.issue_id, blockerIds);
    }
    return map;
  }, [pageIssues]);

  // Direct child IDs per page issue, derived from the relations response.
  const directChildIdsByIssue = useMemo(() => {
    const map = new Map<string, string[]>();
    for (const rel of childRelations ?? []) {
      const list = map.get(rel.target_id) ?? [];
      if (!list.includes(rel.source_id)) list.push(rel.source_id);
      map.set(rel.target_id, list);
    }
    return map;
  }, [childRelations]);

  // Union of neighbor IDs not already in the page set.
  const neighborIds = useMemo(() => {
    const pageIdSet = new Set(pageIssueIds);
    const ids = new Set<string>();
    for (const list of directChildIdsByIssue.values()) {
      for (const id of list) {
        if (!pageIdSet.has(id)) ids.add(id);
      }
    }
    for (const list of directBlockerIdsByIssue.values()) {
      for (const id of list) {
        if (!pageIdSet.has(id)) ids.add(id);
      }
    }
    return Array.from(ids).sort();
  }, [pageIssueIds, directChildIdsByIssue, directBlockerIdsByIssue]);

  const { data: sessions } = useSessions(pageIssueIds);
  const { data: neighborSummaries } = useIssueSummaries(neighborIds);

  const issueMap = useMemo(() => {
    const map = new Map<string, IssueSummaryRecord>();
    for (const issue of pageIssues) {
      map.set(issue.issue_id, issue);
    }
    if (neighborSummaries) {
      for (const issue of neighborSummaries) {
        map.set(issue.issue_id, issue);
      }
    }
    return map;
  }, [pageIssues, neighborSummaries]);

  const sessionsByIssue = useMemo(
    () => groupSessionsByIssue(sessions ?? []),
    [sessions],
  );

  const neighborhoodMap = useMemo(() => {
    const map = new Map<string, IssueNeighborhood>();
    for (const issueId of pageIssueIds) {
      const childIds = directChildIdsByIssue.get(issueId) ?? [];
      const blockerIds = directBlockerIdsByIssue.get(issueId) ?? [];
      const children: NeighborStatus[] = [];
      for (const id of childIds) {
        const rec = issueMap.get(id);
        if (!rec) continue;
        children.push({ id, status: rec.issue.status });
      }
      const blockers: NeighborStatus[] = [];
      for (const id of blockerIds) {
        const rec = issueMap.get(id);
        if (!rec) continue;
        blockers.push({ id, status: rec.issue.status });
      }
      if (children.length === 0 && blockers.length === 0) continue;
      map.set(issueId, { children, blockers });
    }
    return map;
  }, [pageIssueIds, directChildIdsByIssue, directBlockerIdsByIssue, issueMap]);

  // Only show loading state on the very first fetch when no data (real or
  // placeholder) is available. With keepPreviousData, subsequent refetches
  // caused by changed query keys will still have placeholder data, so we
  // avoid flashing the shimmer animation.
  const isLoading =
    (pageIssueIds.length > 0 && childRelations === undefined) ||
    (pageIssueIds.length > 0 && sessions === undefined) ||
    (neighborIds.length > 0 && neighborSummaries === undefined);

  return {
    neighborhoodMap,
    sessionsByIssue,
    isLoading,
  };
}
