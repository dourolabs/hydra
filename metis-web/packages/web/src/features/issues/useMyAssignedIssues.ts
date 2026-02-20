import { useMemo } from "react";
import type { IssueVersionRecord } from "@metis/api";
import { useIssues } from "./useIssues";

export function useMyAssignedIssues(username: string) {
  const query = useIssues();

  const data = useMemo(() => {
    if (!query.data || !username) return [];
    return query.data.filter(
      (r: IssueVersionRecord) =>
        r.issue.assignee === username &&
        r.issue.status !== "closed" &&
        r.issue.status !== "dropped" &&
        r.issue.status !== "rejected" &&
        !r.issue.deleted,
    );
  }, [query.data, username]);

  return { ...query, data };
}
