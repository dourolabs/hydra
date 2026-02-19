import { useQuery } from "@tanstack/react-query";
import { fetchJobsByIssue, toJob } from "../../api/jobs";

export function useJobsByIssue(issueId: string) {
  return useQuery({
    queryKey: ["jobs", issueId],
    queryFn: () => fetchJobsByIssue(issueId),
    select: (data) => data.jobs.map(toJob),
    enabled: !!issueId,
  });
}
