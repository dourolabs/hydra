import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import type {
  DocumentSummaryRecord,
  IssueSummaryRecord,
  PatchSummaryRecord,
  SessionSummaryRecord,
} from "@hydra/api";
import { hydraIdKind } from "@hydra/api";
import { apiClient } from "../../api/client";

export interface ReferencedArtifactsResult {
  issues: IssueSummaryRecord[];
  patches: PatchSummaryRecord[];
  documents: DocumentSummaryRecord[];
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
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

  // Mirror IssueRelatedIssues: use queryKey shape ["sessions", "batch", ids]
  // so useSSE's broad invalidation on session_* events gives us live updates
  // for free.
  const sessionsQuery = useQuery({
    queryKey: ["sessions", "batch", issueIdsParam],
    queryFn: () => apiClient.listSessions({ spawned_from_ids: issueIdsParam }),
    enabled: issueIds.length > 0,
    staleTime: 30_000,
    select: (data) => data.sessions,
  });

  const patchIdsParam = patchIds.join(",");
  const patchesQuery = useQuery({
    queryKey: ["chatRelated", "referencedPatches", patchIdsParam],
    queryFn: () => apiClient.listPatches({ ids: patchIdsParam, limit: patchIds.length }),
    enabled: patchIds.length > 0,
    staleTime: 30_000,
    select: (data) => data.patches,
  });

  const documentIdsParam = documentIds.join(",");
  const documentsQuery = useQuery({
    queryKey: ["chatRelated", "referencedDocuments", documentIdsParam],
    queryFn: () =>
      apiClient.listDocuments({ ids: documentIdsParam, limit: documentIds.length }),
    enabled: documentIds.length > 0,
    staleTime: 30_000,
    select: (data) => data.documents,
  });

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

  const documentsMap = useMemo(() => {
    const map = new Map<string, DocumentSummaryRecord>();
    for (const doc of documentsQuery.data ?? []) {
      map.set(doc.document_id, doc);
    }
    return map;
  }, [documentsQuery.data]);

  const orderedDocuments = useMemo(() => {
    const out: DocumentSummaryRecord[] = [];
    for (const id of documentIds) {
      const doc = documentsMap.get(id);
      if (doc) out.push(doc);
    }
    return out;
  }, [documentIds, documentsMap]);

  const sessionsByIssue = useMemo(() => {
    const map = new Map<string, SessionSummaryRecord[]>();
    for (const session of sessionsQuery.data ?? []) {
      const sid = session.session.spawned_from;
      if (!sid) continue;
      const list = map.get(sid) ?? [];
      list.push(session);
      map.set(sid, list);
    }
    return map;
  }, [sessionsQuery.data]);

  const isLoading =
    relationsQuery.isLoading ||
    (issueIds.length > 0 && issuesQuery.isLoading) ||
    (issueIds.length > 0 && sessionsQuery.isLoading) ||
    (patchIds.length > 0 && patchesQuery.isLoading) ||
    (documentIds.length > 0 && documentsQuery.isLoading);

  const error =
    relationsQuery.error ??
    issuesQuery.error ??
    sessionsQuery.error ??
    patchesQuery.error ??
    documentsQuery.error ??
    null;

  return {
    issues: orderedIssues,
    patches: orderedPatches,
    documents: orderedDocuments,
    sessionsByIssue,
    isLoading,
    error,
  };
}
