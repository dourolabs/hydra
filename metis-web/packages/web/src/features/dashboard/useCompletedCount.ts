import { useMemo } from "react";
import type { IssueSummaryRecord } from "@metis/api";
import { buildIssueTree } from "../issues/useIssues";

const TERMINAL_STATUSES = new Set(["closed", "failed", "dropped", "rejected"]);

export function useCompletedCount(
  issues: IssueSummaryRecord[] | undefined,
  username: string,
): number {
  return useMemo(() => {
    if (!issues) return 0;
    const tree = buildIssueTree(issues);
    return tree.filter(
      (root) =>
        !root.hardBlocked &&
        root.issue.issue.creator === username &&
        TERMINAL_STATUSES.has(root.issue.issue.status),
    ).length;
  }, [issues, username]);
}
