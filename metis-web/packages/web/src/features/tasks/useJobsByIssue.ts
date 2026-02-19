import { useQuery } from "@tanstack/react-query";
import { fetchJobsByIssue } from "../../api/jobs";

export function useJobsByIssue(issueId: string) {
  return useQuery({
    queryKey: ["jobs", issueId],
    queryFn: () => fetchJobsByIssue(issueId),
    select: (data) => data.jobs,
    enabled: !!issueId,
  });
}
