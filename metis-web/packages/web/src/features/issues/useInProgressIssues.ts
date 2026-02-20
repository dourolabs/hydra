import { useMemo } from "react";
import type { IssueVersionRecord } from "@metis/api";
import { useIssues } from "./useIssues";

export function useInProgressIssues() {
  const query = useIssues();

  const data = useMemo(() => {
    if (!query.data) return [];
    return query.data.filter(
      (r: IssueVersionRecord) =>
        r.issue.status === "in-progress" && !r.issue.deleted,
    );
  }, [query.data]);

  return { ...query, data };
}
