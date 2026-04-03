import { useMemo } from "react";
import { useQueries } from "@tanstack/react-query";
import type { IssueSummaryRecord, LabelRecord, SearchIssuesQuery } from "@hydra/api";
import { apiClient } from "../../api/client";

/**
 * For each label, fetch its issues independently of the main dashboard filter.
 * Returns a map from label ID to the list of issues with that label.
 * Uses cursor-based pagination to fetch all issues (up to MAX_ISSUES cap).
 */
const EMPTY_LABELS: LabelRecord[] = [];
const PAGE_SIZE = 200;
const MAX_ISSUES = 1000;

async function fetchAllLabelIssues(labelId: string): Promise<IssueSummaryRecord[]> {
  const allIssues: IssueSummaryRecord[] = [];
  let cursor: string | undefined;

  while (allIssues.length < MAX_ISSUES) {
    const query: Partial<SearchIssuesQuery> = {
      labels: labelId,
      limit: PAGE_SIZE,
    };
    if (cursor) {
      query.cursor = cursor;
    }
    const resp = await apiClient.listIssues(query);
    allIssues.push(...resp.issues);
    if (!resp.next_cursor || resp.issues.length === 0) {
      break;
    }
    cursor = resp.next_cursor;
  }

  return allIssues;
}

export function useLabelIssues(labels: LabelRecord[] | undefined) {
  const labelList = labels ?? EMPTY_LABELS;

  const queries = useQueries({
    queries: labelList.map((label) => ({
      queryKey: ["labelIssues", label.label_id],
      queryFn: (): Promise<IssueSummaryRecord[]> => fetchAllLabelIssues(label.label_id),
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
