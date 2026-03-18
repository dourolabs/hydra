import { useQuery } from "@tanstack/react-query";
import { apiClient } from "../../api/client";

export function useDocument(documentId: string) {
  return useQuery({
    queryKey: ["document", documentId],
    queryFn: () => apiClient.getDocument(documentId),
    enabled: !!documentId,
  });
}
