import { useMemo } from "react";
import type { IssueSummaryRecord, JobSummaryRecord } from "@metis/api";
import { buildIssueTree } from "../issues/useIssues";
import { pruneTree } from "./watchingUtils";

export function useWatchingCount(
  issues: IssueSummaryRecord[] | undefined,
  jobsByIssue: Map<string, JobSummaryRecord[]> | undefined,
  username: string,
): number {
  return useMemo(() => {
    if (!issues) return 0;
    const jobs = jobsByIssue ?? new Map<string, JobSummaryRecord[]>();
    const tree = buildIssueTree(issues);
    return tree.filter((root) => !root.hardBlocked && root.issue.issue.creator === username && pruneTree(root, jobs) !== null).length;
  }, [issues, jobsByIssue, username]);
}
