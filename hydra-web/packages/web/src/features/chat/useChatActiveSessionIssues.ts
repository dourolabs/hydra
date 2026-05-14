import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import type { IssueSummaryRecord, SessionSummaryRecord } from "@hydra/api";
import { apiClient } from "../../api/client";

const MAX_DISPLAYED = 25;

export interface ActiveSessionIssuesResult {
  issues: IssueSummaryRecord[];
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  isLoading: boolean;
}

/**
 * Section 1: Issues that currently have at least one running or pending
 * session. Sources sessions first, groups by `spawned_from`, then batch-fetches
 * the owning issues. Sorted by most recent session timestamp; capped at 25.
 */
export function useChatActiveSessionIssues(): ActiveSessionIssuesResult {
  const sessionsQuery = useQuery({
    queryKey: ["chatRelated", "activeSessions"],
    queryFn: () =>
      apiClient.listSessions({ status: "running,pending", limit: 100 }),
    staleTime: 30_000,
  });

  const { sessionsByIssue, orderedIssueIds } = useMemo(() => {
    const sessions = sessionsQuery.data?.sessions ?? [];
    const byIssue = new Map<string, SessionSummaryRecord[]>();
    for (const session of sessions) {
      const issueId = session.session.spawned_from;
      if (!issueId) continue;
      const list = byIssue.get(issueId);
      if (list) {
        list.push(session);
      } else {
        byIssue.set(issueId, [session]);
      }
    }

    const ordered = Array.from(byIssue.entries())
      .map(([issueId, sess]) => ({
        issueId,
        latest: sess.reduce((max, s) => (s.timestamp > max ? s.timestamp : max), ""),
      }))
      .sort((a, b) => (a.latest < b.latest ? 1 : a.latest > b.latest ? -1 : 0))
      .slice(0, MAX_DISPLAYED)
      .map((e) => e.issueId);

    return { sessionsByIssue: byIssue, orderedIssueIds: ordered };
  }, [sessionsQuery.data]);

  const idsParam = orderedIssueIds.join(",");
  const issuesQuery = useQuery({
    queryKey: ["chatRelated", "activeSessionIssues", idsParam],
    queryFn: () => apiClient.listIssues({ ids: idsParam, limit: orderedIssueIds.length }),
    enabled: orderedIssueIds.length > 0,
    staleTime: 30_000,
    select: (data) => data.issues,
  });

  const issueMap = new Map<string, IssueSummaryRecord>();
  for (const issue of issuesQuery.data ?? []) {
    issueMap.set(issue.issue_id, issue);
  }

  const issues: IssueSummaryRecord[] = [];
  for (const id of orderedIssueIds) {
    const issue = issueMap.get(id);
    if (issue) issues.push(issue);
  }

  return {
    issues,
    sessionsByIssue,
    isLoading:
      sessionsQuery.isLoading ||
      (orderedIssueIds.length > 0 && issuesQuery.isLoading),
  };
}
