import { useQueries, useQuery } from "@tanstack/react-query";
import type { IssueVersionRecord } from "@hydra/api";
import { apiClient } from "../../api/client";

export function useIssue(issueId: string) {
  return useQuery({
    queryKey: ["issue", issueId],
    queryFn: () => apiClient.getIssue(issueId),
    enabled: !!issueId,
  });
}

/**
 * Batch-fetch a set of issues by id, sharing the React Query cache with
 * any individual `useIssue` calls for the same ids. Returns a map of
 * id → record (records absent from the map are still loading or failed).
 *
 * The returned `Map` is rebuilt on every render. Construction is O(n) over
 * the requested ids and callers typically wrap their derived values in
 * their own `useMemo`, so referential stability here would add no value.
 */
export function useIssuesByIds(
  issueIds: string[],
): Map<string, IssueVersionRecord> {
  const queries = useQueries({
    queries: issueIds.map((id) => ({
      queryKey: ["issue", id],
      queryFn: () => apiClient.getIssue(id),
      enabled: !!id,
    })),
  });

  const map = new Map<string, IssueVersionRecord>();
  for (let i = 0; i < issueIds.length; i++) {
    const data = queries[i]?.data;
    if (data) map.set(issueIds[i], data);
  }
  return map;
}
