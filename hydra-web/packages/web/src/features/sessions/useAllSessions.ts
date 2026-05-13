import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useAllSessions() {
  return useQuery({
    queryKey: ["sessions", "all"],
    queryFn: () => apiClient.listSessions(),
    select: (data) => data.sessions,
  });
}
