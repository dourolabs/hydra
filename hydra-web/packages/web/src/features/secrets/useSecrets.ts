import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useSecrets(username: string | null) {
  return useQuery({
    queryKey: ["secrets", username],
    queryFn: () => apiClient.listSecrets(username!),
    enabled: username !== null,
  });
}
