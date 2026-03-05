import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useSecrets() {
  return useQuery({
    queryKey: ["secrets"],
    queryFn: () => apiClient.listSecrets(),
  });
}
