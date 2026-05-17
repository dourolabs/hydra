import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import type { DocumentSummaryRecord } from "@hydra/api";
import { apiClient } from "../../api/client";

/**
 * Fetch documents linked to an issue via the `has-document` relation.
 * Queries relations first, then batch-fetches the document summaries.
 *
 * The relations query key is shaped so the SSE `['relations', 'has-document']`
 * invalidation in useSSE refreshes it.
 */
export function useIssueDocuments(issueId: string) {
  const relationsQuery = useQuery({
    queryKey: ["relations", "has-document", issueId],
    queryFn: () =>
      apiClient.listRelations({
        source_id: issueId,
        rel_type: "has-document",
      }),
    enabled: !!issueId,
    staleTime: 30_000,
    select: (data) => data.relations,
  });

  const documentIds = useMemo(
    () => relationsQuery.data?.map((rel) => rel.target_id) ?? [],
    [relationsQuery.data],
  );

  const idsParam = documentIds.join(",");
  const documentsQuery = useQuery({
    queryKey: ["documents", "batch", idsParam],
    queryFn: () =>
      apiClient.listDocuments({ ids: idsParam, limit: documentIds.length }),
    select: (resp): DocumentSummaryRecord[] => resp.documents,
    enabled: documentIds.length > 0,
    staleTime: 30_000,
  });

  const orderedDocuments = useMemo(() => {
    if (documentIds.length === 0) return [];
    const map = new Map<string, DocumentSummaryRecord>();
    for (const doc of documentsQuery.data ?? []) {
      map.set(doc.document_id, doc);
    }
    const out: DocumentSummaryRecord[] = [];
    for (const id of documentIds) {
      const doc = map.get(id);
      if (doc) out.push(doc);
    }
    return out;
  }, [documentIds, documentsQuery.data]);

  const isLoading =
    relationsQuery.isLoading || (documentIds.length > 0 && documentsQuery.isLoading);
  const error = relationsQuery.error ?? documentsQuery.error ?? null;

  return {
    data: orderedDocuments,
    isLoading,
    error,
  };
}
