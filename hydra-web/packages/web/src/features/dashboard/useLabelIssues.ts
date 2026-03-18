import { useMemo } from "react";
import { useQueries } from "@tanstack/react-query";
import type { IssueSummaryRecord, LabelRecord, SearchIssuesQuery } from "@hydra/api";
import { apiClient } from "../../api/client";

/**
 * For each label, fetch its issues independently of the main dashboard filter.
 * Returns a map from label ID to the list of issues with that label.
 */
export function useLabelIssues(labels: LabelRecord[] | undefined) {
  const labelList = labels ?? [];

  const queries = useQueries({
    queries: labelList.map((label) => ({
      queryKey: ["labelIssues", label.label_id],
      queryFn: async (): Promise<IssueSummaryRecord[]> => {
        const query: Partial<SearchIssuesQuery> = {
          labels: label.label_id,
          limit: 200,
        };
        const resp = await apiClient.listIssues(query);
        return resp.issues;
      },
    })),
  });

  return useMemo(() => {
    const map = new Map<string, IssueSummaryRecord[]>();
    for (let i = 0; i < labelList.length; i++) {
      map.set(labelList[i].label_id, queries[i].data ?? []);
    }
    return map;
  }, [labelList, queries]);
}
