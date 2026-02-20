import { useMemo } from "react";
import type { IssueVersionRecord } from "@metis/api";
import { buildIssueTree } from "../issues/useIssues";
import { containsAssignedOpen } from "./watchingUtils";

export function useWatchingCount(issues: IssueVersionRecord[] | undefined, username: string): number {
  return useMemo(() => {
    if (!issues) return 0;
    const tree = buildIssueTree(issues);

    if (!username) {
      return tree.filter((node) => {
        const status = node.issue.issue.status;
        return status === "open" || status === "in-progress";
      }).length;
    }

    return tree.filter((root) => {
      return (
        root.issue.issue.status === "in-progress" ||
        containsAssignedOpen(root, username)
      );
    }).length;
  }, [issues, username]);
}
