import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useInProgressIssues() {
  return useQuery({
    queryKey: ["issues", "in-progress"],
    queryFn: () => apiClient.listIssues({ status: "in-progress" }),
    select: (data) => data.issues,
  });
}
