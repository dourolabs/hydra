import { useMemo } from "react";
import { useIssues } from "./useIssues";

export function useInProgressIssues() {
  const { data: issues, isLoading, error } = useIssues();

  const inProgressIssues = useMemo(() => {
    if (!issues) return [];
    return issues.filter((record) => record.issue.status === "in-progress");
  }, [issues]);

  return { data: inProgressIssues, isLoading, error };
}
