import { useQuery } from "@tanstack/react-query";
import type { ListDocumentsResponse } from "@hydra/api";
import { apiClient } from "../../api/client";

export function useUncategorizedDocuments(enabled: boolean) {
  return useQuery<ListDocumentsResponse, Error>({
    queryKey: ["uncategorizedDocuments"],
    queryFn: () => apiClient.listDocuments({ limit: 200 }),
    select: (data) => ({
      ...data,
      documents: data.documents.filter((d) => !d.document.path && !d.document.archived),
    }),
    enabled,
  });
}
