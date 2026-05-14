import { useQuery } from "@tanstack/react-query";
import type { IssueSummaryRecord } from "@hydra/api";
import { apiClient } from "../../api/client";

const MAX_DISPLAYED = 25;
const FETCH_LIMIT = 200;

export interface TopLevelIssuesResult {
  issues: IssueSummaryRecord[];
  isLoading: boolean;
}

/**
 * Section 3: Issues that have no `child-of` parent dependency (i.e. "top-level"
 * roots). Server has no such filter today, so we fetch a generous batch and
 * filter client-side. Excludes IDs already shown in earlier sections; capped
 * at 25.
 */
export function useChatTopLevelIssues(excludeIds: Set<string>): TopLevelIssuesResult {
  const query = useQuery({
    queryKey: ["chatRelated", "topLevel"],
    queryFn: () => apiClient.listIssues({ limit: FETCH_LIMIT }),
    staleTime: 30_000,
    select: (data) => data.issues,
  });

  const hasChildOf = (issue: IssueSummaryRecord) =>
    issue.issue.dependencies.some((dep) => dep.type === "child-of");

  const filtered = (query.data ?? [])
    .filter((issue) => !hasChildOf(issue) && !excludeIds.has(issue.issue_id))
    .sort((a, b) => (a.timestamp < b.timestamp ? 1 : a.timestamp > b.timestamp ? -1 : 0))
    .slice(0, MAX_DISPLAYED);

  return { issues: filtered, isLoading: query.isLoading };
}
