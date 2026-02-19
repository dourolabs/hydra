import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useJob(jobId: string) {
  return useQuery({
    queryKey: ["job", jobId],
    queryFn: () => apiClient.getJob(jobId),
    enabled: !!jobId,
  });
}
