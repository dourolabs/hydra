import { useMemo } from "react";
import type { IssueVersionRecord, JobVersionRecord } from "@metis/api";
import { buildIssueTree } from "../issues/useIssues";
import { pruneTree } from "./watchingUtils";

export function useWatchingCount(
  issues: IssueVersionRecord[] | undefined,
  jobsByIssue: Map<string, JobVersionRecord[]> | undefined,
): number {
  return useMemo(() => {
    if (!issues) return 0;
    const jobs = jobsByIssue ?? new Map<string, JobVersionRecord[]>();
    const tree = buildIssueTree(issues);
    return tree.filter((root) => pruneTree(root, jobs) !== null).length;
  }, [issues, jobsByIssue]);
}
