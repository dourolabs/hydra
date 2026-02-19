import { useQuery } from "@tanstack/react-query";
import { fetchJob, toJob } from "../../api/jobs";

export function useJob(jobId: string) {
  return useQuery({
    queryKey: ["job", jobId],
    queryFn: () => fetchJob(jobId),
    select: (data) => toJob(data),
    enabled: !!jobId,
  });
}
