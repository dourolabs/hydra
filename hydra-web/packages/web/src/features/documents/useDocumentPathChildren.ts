import { useQuery } from "@tanstack/react-query";
import type { ListDocumentPathsResponse } from "@hydra/api";
import { apiClient } from "../../api/client";

export function useDocumentPathChildren(
  prefix: string | null,
  enabled = true,
) {
  return useQuery<ListDocumentPathsResponse, Error>({
    queryKey: ["documentPaths", prefix],
    queryFn: () => apiClient.listDocumentPaths({ prefix }),
    enabled,
  });
}
