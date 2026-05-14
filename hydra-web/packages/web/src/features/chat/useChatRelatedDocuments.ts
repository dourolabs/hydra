import { useQuery } from "@tanstack/react-query";
import type { DocumentSummaryRecord } from "@hydra/api";
import { apiClient } from "../../api/client";

const MAX_DISPLAYED = 25;

export interface RelatedDocumentsResult {
  documents: DocumentSummaryRecord[];
  isLoading: boolean;
}

/** Section 4: Most recent documents (capped at 25). */
export function useChatRelatedDocuments(): RelatedDocumentsResult {
  const query = useQuery({
    queryKey: ["chatRelated", "documents"],
    queryFn: () => apiClient.listDocuments({ limit: MAX_DISPLAYED }),
    staleTime: 30_000,
    select: (data) => data.documents,
  });

  return {
    documents: query.data ?? [],
    isLoading: query.isLoading,
  };
}
