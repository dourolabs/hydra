import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useRunningJobs() {
  return useQuery({
    queryKey: ["jobs", "running"],
    queryFn: () => apiClient.listJobs({ status: "running" }),
    select: (data) => data.jobs,
    refetchInterval: 10_000,
  });
}
