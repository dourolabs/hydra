import { useQuery } from "@tanstack/react-query";
import type { ListDocumentsResponse } from "@hydra/api";
import { apiClient } from "../../api/client";

export function useDocumentSummariesUnderPath(
  path: string | null,
  enabled = true,
) {
  return useQuery<ListDocumentsResponse, Error>({
    queryKey: ["documentsUnderPath", path],
    queryFn: () => apiClient.listDocuments({ path_prefix: path! }),
    enabled: enabled && !!path,
  });
}
