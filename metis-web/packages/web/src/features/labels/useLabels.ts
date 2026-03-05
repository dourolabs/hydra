import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useLabels() {
  return useQuery({
    queryKey: ["labels"],
    queryFn: () => apiClient.listLabels(),
    select: (data) => data.labels,
  });
}
