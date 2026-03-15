import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import type { IssueSummaryRecord } from "@metis/api";
import { apiClient } from "../../api/client";

/**
 * Lightweight hook that returns a Set of issue IDs that have active
 * (running or pending) sessions.  Used for badge counting where we need
 * session status for ALL issues, not just the current page.
 */
export function useActiveSessionIssueIds(
  issues: IssueSummaryRecord[],
): { activeIssueIds: Set<string>; isLoading: boolean } {
  const issueIds = useMemo(
    () => issues.map((i) => i.issue_id),
    [issues],
  );

  const spawned_from_ids = issueIds.join(",");

  const { data: sessions, isLoading } = useQuery({
    queryKey: ["sessions", "active-badge", spawned_from_ids],
    queryFn: () =>
      apiClient.listSessions({
        spawned_from_ids,
        status: "running,pending",
      }),
    enabled: issueIds.length > 0,
    staleTime: 30_000,
    select: (data) => data.sessions,
  });

  const activeIssueIds = useMemo(() => {
    const set = new Set<string>();
    if (!sessions) return set;
    for (const session of sessions) {
      const issueId = session.session.spawned_from;
      if (issueId) {
        set.add(issueId);
      }
    }
    return set;
  }, [sessions]);

  return { activeIssueIds, isLoading };
}
