import { useQuery } from "@tanstack/react-query";
import { fetchIssue } from "../../api/issues";

export function useIssue(issueId: string) {
  return useQuery({
    queryKey: ["issue", issueId],
    queryFn: () => fetchIssue(issueId),
    enabled: !!issueId,
  });
}
