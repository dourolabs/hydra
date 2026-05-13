import { useQuery } from "@tanstack/react-query";
import type { ListDocumentsResponse } from "@hydra/api";
import { apiClient } from "../../api/client";

export function useDocumentSummariesAtPath(
  path: string | null,
  enabled = true,
) {
  return useQuery<ListDocumentsResponse, Error>({
    queryKey: ["documentsAtPath", path],
    queryFn: () =>
      apiClient.listDocuments({ path_prefix: path!, path_is_exact: true }),
    enabled: enabled && !!path,
  });
}
