import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import type { DocumentSummaryRecord } from "@hydra/api";
import { apiClient } from "../../api/client";

/**
 * Fetch document summaries by their IDs in a single bulk request.
 * Returns rows ordered to match the input id sequence; ids missing from the
 * response (e.g. deleted documents) are skipped.
 */
export function useDocumentsByIds(documentIds: string[]) {
  const stableIds = useMemo(() => [...documentIds].sort(), [documentIds]);
  const idsCsv = stableIds.join(",");

  const query = useQuery({
    queryKey: ["documentsByIds", idsCsv],
    queryFn: () =>
      apiClient.listDocuments({ ids: idsCsv, limit: stableIds.length }),
    staleTime: 30_000,
    enabled: stableIds.length > 0,
    select: (data) => data.documents,
  });

  const data: DocumentSummaryRecord[] = useMemo(() => {
    const docMap = new Map<string, DocumentSummaryRecord>();
    for (const doc of query.data ?? []) {
      docMap.set(doc.document_id, doc);
    }
    const ordered: DocumentSummaryRecord[] = [];
    for (const id of stableIds) {
      const doc = docMap.get(id);
      if (doc) ordered.push(doc);
    }
    return ordered;
  }, [query.data, stableIds]);

  return {
    data,
    isLoading: stableIds.length > 0 && query.isLoading,
    error: query.error ?? null,
  };
}
