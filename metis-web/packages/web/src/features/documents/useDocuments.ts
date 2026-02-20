import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useDocuments() {
  return useQuery({
    queryKey: ["documents"],
    queryFn: () => apiClient.listDocuments(),
    select: (data) => data.documents,
  });
}
