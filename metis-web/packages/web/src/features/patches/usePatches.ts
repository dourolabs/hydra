import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function usePatches() {
  return useQuery({
    queryKey: ["patches"],
    queryFn: () => apiClient.listPatches(),
    select: (data) => data.patches,
  });
}
