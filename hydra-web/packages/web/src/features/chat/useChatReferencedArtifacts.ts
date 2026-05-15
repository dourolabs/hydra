import { useMemo } from "react";
import { useQueries, useQuery } from "@tanstack/react-query";
import type { DocumentVersionRecord, IssueSummaryRecord, PatchSummaryRecord } from "@hydra/api";
import { hydraIdKind } from "@hydra/api";
import { apiClient } from "../../api/client";

export interface ReferencedArtifactsResult {
  issues: IssueSummaryRecord[];
  patches: PatchSummaryRecord[];
  documents: DocumentVersionRecord[];
  isLoading: boolean;
  error: unknown;
}

const MAX_PER_BUCKET = 33;

function bucketByPrefix(ids: string[]): {
  issueIds: string[];
  patchIds: string[];
  documentIds: string[];
} {
  const issueIds: string[] = [];
  const patchIds: string[] = [];
  const documentIds: string[] = [];
  for (const id of ids) {
    switch (hydraIdKind(id)) {
      case "issue":
        issueIds.push(id);
        break;
      case "patch":
        patchIds.push(id);
        break;
      case "document":
        documentIds.push(id);
        break;
    }
  }
  return {
    issueIds: issueIds.slice(0, MAX_PER_BUCKET),
    patchIds: patchIds.slice(0, MAX_PER_BUCKET),
    documentIds: documentIds.slice(0, MAX_PER_BUCKET),
  };
}

/**
 * Fetch artifacts the given conversation `RefersTo` via a single backend
 * pre-filter on the relations table. Buckets relation rows by target_id prefix
 * (i-/p-/d-) and batch-fetches details for each kind.
 */
export function useChatReferencedArtifacts(conversationId: string): ReferencedArtifactsResult {
  const relationsQuery = useQuery({
    queryKey: ["chatRelated", "refers_to", conversationId],
    queryFn: () =>
      apiClient.listRelations({
        source_id: conversationId,
        rel_type: "refers_to",
      }),
    enabled: !!conversationId,
    staleTime: 30_000,
    select: (data) => data.relations,
  });

  const targetIds = useMemo(
    () => relationsQuery.data?.map((rel) => rel.target_id) ?? [],
    [relationsQuery.data],
  );

  const { issueIds, patchIds, documentIds } = useMemo(() => bucketByPrefix(targetIds), [targetIds]);

  const issueIdsParam = issueIds.join(",");
  const issuesQuery = useQuery({
    queryKey: ["chatRelated", "referencedIssues", issueIdsParam],
    queryFn: () => apiClient.listIssues({ ids: issueIdsParam, limit: issueIds.length }),
    enabled: issueIds.length > 0,
    staleTime: 30_000,
    select: (data) => data.issues,
  });

  const patchIdsParam = patchIds.join(",");
  const patchesQuery = useQuery({
    queryKey: ["chatRelated", "referencedPatches", patchIdsParam],
    queryFn: () => apiClient.listPatches({ ids: patchIdsParam, limit: patchIds.length }),
    enabled: patchIds.length > 0,
    staleTime: 30_000,
    select: (data) => data.patches,
  });

  const documentQueries = useQueries({
    queries: documentIds.map((id) => ({
      queryKey: ["document", id],
      queryFn: () => apiClient.getDocument(id),
      staleTime: 30_000,
      enabled: !!id,
    })),
  });

  const documents = useMemo(() => {
    const docMap = new Map<string, DocumentVersionRecord>();
    for (const q of documentQueries) {
      if (q.data) docMap.set(q.data.document_id, q.data);
    }
    const ordered: DocumentVersionRecord[] = [];
    for (const id of documentIds) {
      const doc = docMap.get(id);
      if (doc) ordered.push(doc);
    }
    return ordered;
  }, [documentQueries, documentIds]);

  const issuesMap = useMemo(() => {
    const map = new Map<string, IssueSummaryRecord>();
    for (const issue of issuesQuery.data ?? []) {
      map.set(issue.issue_id, issue);
    }
    return map;
  }, [issuesQuery.data]);

  const orderedIssues = useMemo(() => {
    const out: IssueSummaryRecord[] = [];
    for (const id of issueIds) {
      const issue = issuesMap.get(id);
      if (issue) out.push(issue);
    }
    return out;
  }, [issueIds, issuesMap]);

  const patchesMap = useMemo(() => {
    const map = new Map<string, PatchSummaryRecord>();
    for (const patch of patchesQuery.data ?? []) {
      map.set(patch.patch_id, patch);
    }
    return map;
  }, [patchesQuery.data]);

  const orderedPatches = useMemo(() => {
    const out: PatchSummaryRecord[] = [];
    for (const id of patchIds) {
      const patch = patchesMap.get(id);
      if (patch) out.push(patch);
    }
    return out;
  }, [patchIds, patchesMap]);

  const isLoading =
    relationsQuery.isLoading ||
    (issueIds.length > 0 && issuesQuery.isLoading) ||
    (patchIds.length > 0 && patchesQuery.isLoading) ||
    documentQueries.some((q) => q.isLoading);

  const error =
    relationsQuery.error ??
    issuesQuery.error ??
    patchesQuery.error ??
    documentQueries.find((q) => q.error)?.error ??
    null;

  return {
    issues: orderedIssues,
    patches: orderedPatches,
    documents,
    isLoading,
    error,
  };
}
