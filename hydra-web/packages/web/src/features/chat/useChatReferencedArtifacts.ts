import { useMemo } from "react";
import { keepPreviousData, useInfiniteQuery, useQuery } from "@tanstack/react-query";
import type {
  DocumentSummaryRecord,
  IssueSummaryRecord,
  ListDocumentsResponse,
  ListIssuesResponse,
  ListPatchesResponse,
  PatchSummaryRecord,
  SessionSummaryRecord,
} from "@hydra/api";
import { hydraIdKind } from "@hydra/api";
import { apiClient } from "../../api/client";

export interface ReferencedArtifactsPagination {
  issues: boolean;
  patches: boolean;
  documents: boolean;
}

export interface ReferencedArtifactsFetchers {
  issues: () => void;
  patches: () => void;
  documents: () => void;
}

export interface ReferencedArtifactsResult {
  issues: IssueSummaryRecord[];
  patches: PatchSummaryRecord[];
  documents: DocumentSummaryRecord[];
  sessionsByIssue: Map<string, SessionSummaryRecord[]>;
  isLoading: boolean;
  error: unknown;
  hasNextPage: ReferencedArtifactsPagination;
  isFetchingNextPage: ReferencedArtifactsPagination;
  fetchNextPage: ReferencedArtifactsFetchers;
}

const PAGE_SIZE = 25;

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
  return { issueIds, patchIds, documentIds };
}

/**
 * Fetch artifacts the given conversation `RefersTo` via a single backend
 * pre-filter on the relations table. Buckets relation rows by target_id prefix
 * (i-/p-/d-) and batch-fetches details for each kind with cursor-paginated
 * `useInfiniteQuery` calls so the per-section "Load more" button can extend
 * each list lazily.
 */
export function useChatReferencedArtifacts(conversationId: string): ReferencedArtifactsResult {
  const relationsQuery = useQuery({
    queryKey: ["chatRelated", "refers-to", conversationId],
    queryFn: () =>
      apiClient.listRelations({
        source_id: conversationId,
        rel_type: "refers-to",
      }),
    enabled: !!conversationId,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
    select: (data) => data.relations,
  });

  const targetIds = useMemo(
    () => relationsQuery.data?.map((rel) => rel.target_id) ?? [],
    [relationsQuery.data],
  );

  const { issueIds, patchIds, documentIds } = useMemo(() => bucketByPrefix(targetIds), [targetIds]);

  const issueIdsParam = issueIds.join(",");
  const issuesQuery = useInfiniteQuery<ListIssuesResponse, Error>({
    queryKey: ["chatRelated", "referencedIssues", issueIdsParam],
    queryFn: ({ pageParam }) =>
      apiClient.listIssues({
        ids: issueIdsParam,
        limit: PAGE_SIZE,
        cursor: (pageParam as string | undefined) ?? null,
      }),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    enabled: issueIds.length > 0,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
  });

  const issues = useMemo(
    () => issuesQuery.data?.pages.flatMap((p) => p.issues) ?? [],
    [issuesQuery.data],
  );

  // Sessions stay in lockstep with the issues fetched so far. Keep the
  // ["sessions", "batch", ids] queryKey shape so useSSE's broad invalidation
  // on session_* events refreshes it.
  const fetchedIssueIds = useMemo(() => issues.map((i) => i.issue_id), [issues]);
  const sessionsIdsParam = fetchedIssueIds.join(",");
  const sessionsQuery = useQuery({
    queryKey: ["sessions", "batch", sessionsIdsParam],
    queryFn: () => apiClient.listSessions({ spawned_from_ids: sessionsIdsParam }),
    enabled: fetchedIssueIds.length > 0,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
    select: (data) => data.sessions,
  });

  const patchIdsParam = patchIds.join(",");
  const patchesQuery = useInfiniteQuery<ListPatchesResponse, Error>({
    queryKey: ["chatRelated", "referencedPatches", patchIdsParam],
    queryFn: ({ pageParam }) =>
      apiClient.listPatches({
        ids: patchIdsParam,
        limit: PAGE_SIZE,
        cursor: (pageParam as string | undefined) ?? null,
      }),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    enabled: patchIds.length > 0,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
  });

  const patches = useMemo(
    () => patchesQuery.data?.pages.flatMap((p) => p.patches) ?? [],
    [patchesQuery.data],
  );

  const documentIdsParam = documentIds.join(",");
  const documentsQuery = useInfiniteQuery<ListDocumentsResponse, Error>({
    queryKey: ["chatRelated", "referencedDocuments", documentIdsParam],
    queryFn: ({ pageParam }) =>
      apiClient.listDocuments({
        ids: documentIdsParam,
        limit: PAGE_SIZE,
        cursor: (pageParam as string | undefined) ?? null,
      }),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
    enabled: documentIds.length > 0,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
  });

  const documents = useMemo(
    () => documentsQuery.data?.pages.flatMap((p) => p.documents) ?? [],
    [documentsQuery.data],
  );

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
    (fetchedIssueIds.length > 0 && sessionsQuery.isLoading) ||
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
    issues,
    patches,
    documents,
    sessionsByIssue,
    isLoading,
    error,
    hasNextPage: {
      issues: !!issuesQuery.hasNextPage,
      patches: !!patchesQuery.hasNextPage,
      documents: !!documentsQuery.hasNextPage,
    },
    isFetchingNextPage: {
      issues: issuesQuery.isFetchingNextPage,
      patches: patchesQuery.isFetchingNextPage,
      documents: documentsQuery.isFetchingNextPage,
    },
    fetchNextPage: {
      issues: () => {
        void issuesQuery.fetchNextPage();
      },
      patches: () => {
        void patchesQuery.fetchNextPage();
      },
      documents: () => {
        void documentsQuery.fetchNextPage();
      },
    },
  };
}
