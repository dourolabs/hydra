import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useSessionLogs(sessionId: string, enabled: boolean) {
  return useQuery({
    queryKey: ["sessionLogs", sessionId],
    queryFn: () => apiClient.getSessionLogs(sessionId).then((r) => r.text()),
    enabled: !!sessionId && enabled,
  });
}
