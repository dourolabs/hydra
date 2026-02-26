import { useMemo } from "react";
import type { IssueSummaryRecord } from "@metis/api";
import { buildIssueTree } from "../issues/useIssues";
import { TERMINAL_STATUSES } from "../../utils/statusMapping";

export function useCompletedCount(
  issues: IssueSummaryRecord[] | undefined,
  username: string,
): number {
  return useMemo(() => {
    if (!issues) return 0;
    const tree = buildIssueTree(issues);
    return tree.filter(
      (root) =>
        root.issue.issue.creator === username &&
        TERMINAL_STATUSES.has(root.issue.issue.status),
    ).length;
  }, [issues, username]);
}
