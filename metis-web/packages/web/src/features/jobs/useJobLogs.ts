import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useJobLogs(jobId: string, enabled: boolean) {
  return useQuery({
    queryKey: ["jobLogs", jobId],
    queryFn: () => apiClient.getJobLogs(jobId).then((r) => r.text()),
    enabled: !!jobId && enabled,
  });
}
