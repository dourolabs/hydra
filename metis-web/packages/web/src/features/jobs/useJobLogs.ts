import { useQuery } from "@tanstack/react-query";
import { fetchJobLogs } from "../../api/jobs";

export function useJobLogs(jobId: string, enabled: boolean) {
  return useQuery({
    queryKey: ["jobLogs", jobId],
    queryFn: () => fetchJobLogs(jobId),
    enabled: !!jobId && enabled,
  });
}
