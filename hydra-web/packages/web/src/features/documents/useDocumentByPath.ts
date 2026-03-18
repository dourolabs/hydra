import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useDocumentByPath(path: string | null) {
  return useQuery({
    queryKey: ["document", "path", path],
    queryFn: () => apiClient.getDocumentByPath(path!),
    enabled: !!path,
  });
}
