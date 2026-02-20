import { useMemo } from "react";
import type { IssueVersionRecord } from "@metis/api";
import { buildIssueTree } from "../issues/useIssues";

export function useWatchingCount(issues: IssueVersionRecord[] | undefined): number {
  return useMemo(() => {
    if (!issues) return 0;
    const tree = buildIssueTree(issues);
    return tree.filter((node) => {
      const status = node.issue.issue.status;
      return status === "open" || status === "in-progress";
    }).length;
  }, [issues]);
}
