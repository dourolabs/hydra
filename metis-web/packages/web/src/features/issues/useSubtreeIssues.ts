import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import type { IssueSummaryRecord } from "@metis/api";
import { apiClient } from "../../api/client";

/**
 * Fetch all transitive children of a root issue using the relationships API,
 * then batch-fetch their details. Returns the root issue + all descendants.
 *
 * Replaces the old pattern of fetching ALL issues and traversing locally.
 */
export function useSubtreeIssues(rootIssueId: string | null) {
  // Step 1: Get transitive child-of relations for the root issue
  const { data: relations, isLoading: relLoading } = useQuery({
    queryKey: ["relations", "child-of", "transitive", rootIssueId],
    queryFn: () =>
      apiClient.listRelations({
        target_ids: rootIssueId!,
        rel_type: "child-of",
        transitive: true,
      }),
    enabled: !!rootIssueId,
    staleTime: 30_000,
    select: (data) => data.relations,
  });

  // Collect all descendant IDs
  const descendantIds = useMemo(() => {
    if (!relations) return [];
    const ids = new Set<string>();
    for (const rel of relations) {
      ids.add(rel.source_id);
    }
    return Array.from(ids);
  }, [relations]);

  // Step 2: Batch fetch descendant issue details
  const allIds = useMemo(() => {
    if (!rootIssueId) return [];
    return [rootIssueId, ...descendantIds];
  }, [rootIssueId, descendantIds]);

  const ids = allIds.join(",");
  const { data: issues, isLoading: issuesLoading } = useQuery({
    queryKey: ["issues", "batch", ids],
    queryFn: () => apiClient.listIssues({ ids }),
    enabled: allIds.length > 0,
    staleTime: 30_000,
    select: (data) => data.issues,
  });

  const isLoading = relLoading || issuesLoading;

  return {
    data: issues as IssueSummaryRecord[] | undefined,
    isLoading,
  };
}
