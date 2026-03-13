import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useSession(sessionId: string) {
  return useQuery({
    queryKey: ["session", sessionId],
    queryFn: () => apiClient.getSession(sessionId),
    enabled: !!sessionId,
  });
}
