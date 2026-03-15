import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import { apiClient } from "../../api/client";

/**
 * Lightweight hook that returns a Set of issue IDs that have active
 * (running or pending) sessions.  Used for badge counting where we need
 * to know which issues have active sessions across the entire issue set,
 * not just the current page.
 */
export function useActiveSessionIssueIds(): {
  activeIssueIds: Set<string>;
  isLoading: boolean;
} {
  const { data: sessions, isLoading } = useQuery({
    queryKey: ["sessions", "active-badge"],
    queryFn: () =>
      apiClient.listSessions({
        status: "running,pending",
      }),
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
