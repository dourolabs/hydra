import { useQuery } from "@tanstack/react-query";
import type { IssueSummaryRecord } from "@hydra/api";
import { apiClient } from "../../api/client";
import { useUsername } from "../auth/useUsername";

const MAX_DISPLAYED = 25;
const FETCH_LIMIT = 50;

export interface AttentionIssuesResult {
  issues: IssueSummaryRecord[];
  isLoading: boolean;
}

/**
 * Section 2: Open issues assigned to the current user. Filters out issues
 * already shown in the active-sessions section to avoid duplication. Capped
 * at 25.
 */
export function useChatAttentionIssues(excludeIds: Set<string>): AttentionIssuesResult {
  const username = useUsername();

  const query = useQuery({
    queryKey: ["chatRelated", "attention", username],
    queryFn: () =>
      apiClient.listIssues({
        assignee: username,
        status: "open",
        limit: FETCH_LIMIT,
      }),
    enabled: !!username,
    staleTime: 30_000,
    select: (data) => data.issues,
  });

  const filtered = (query.data ?? [])
    .filter((issue) => !excludeIds.has(issue.issue_id))
    .slice(0, MAX_DISPLAYED);

  return {
    issues: filtered,
    isLoading: !!username && query.isLoading,
  };
}
