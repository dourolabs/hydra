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
  // Relations query is keyed directly by `issueId`; any key change is a
  // navigation, so don't `keepPreviousData` here (would leak the previous
  // issue's relations into the new issue view).
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
  // Include `issueId` in the queryKey and gate `placeholderData` on it so we
  // only keep stale data for refetches within the same issue. The
  // ["documents", "batch"] prefix is preserved for SSE invalidation.
  const documentsQuery = useQuery({
    queryKey: ["documents", "batch", idsParam, issueId],
    queryFn: () =>
      apiClient.listDocuments({ ids: idsParam, limit: documentIds.length }),
    select: (resp): DocumentSummaryRecord[] => resp.documents,
    enabled: documentIds.length > 0,
    staleTime: 30_000,
    placeholderData: (previousData, previousQuery) =>
      previousQuery?.queryKey[3] === issueId ? previousData : undefined,
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
