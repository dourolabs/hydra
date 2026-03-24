import { useState, useMemo } from "react";
import { useInfiniteQuery } from "@tanstack/react-query";
import { Panel, Button } from "@hydra/ui";
import type { ListDocumentsResponse } from "@hydra/api";
import { apiClient } from "../api/client";
import { LoadingState } from "../components/LoadingState/LoadingState";
import { ErrorState } from "../components/ErrorState/ErrorState";
import { EmptyState } from "../components/EmptyState/EmptyState";
import { DocumentRow } from "../features/documents/DocumentRow";
import { DocumentCreateModal } from "../features/documents/DocumentCreateModal";
import { groupDocumentsByPrefix } from "../features/documents/utils";
import styles from "./DocumentsPage.module.css";

const PAGE_SIZE = 50;

function usePaginatedDocuments() {
  return useInfiniteQuery<ListDocumentsResponse, Error>({
    queryKey: ["paginatedDocuments"],
    queryFn: ({ pageParam }) =>
      apiClient.listDocuments({
        limit: PAGE_SIZE,
        ...(pageParam ? { cursor: pageParam as string } : {}),
      }),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (lastPage) => lastPage.next_cursor ?? undefined,
  });
}

export function DocumentsPage() {
  const {
    data: paginatedData,
    isLoading,
    error,
    refetch,
    fetchNextPage,
    hasNextPage,
    isFetchingNextPage,
  } = usePaginatedDocuments();
  const [createOpen, setCreateOpen] = useState(false);

  const documents = useMemo(
    () => paginatedData?.pages.flatMap((page) => page.documents) ?? [],
    [paginatedData],
  );

  const groups = useMemo(() => groupDocumentsByPrefix(documents), [documents]);

  return (
    <div className={styles.page}>
      <div className={styles.pageHeader}>
        <Button variant="primary" size="sm" onClick={() => setCreateOpen(true)}>
          New Document
        </Button>
      </div>

      {isLoading && <LoadingState />}

      {error && (
        <ErrorState
          message={`Failed to load documents: ${(error as Error).message}`}
          onRetry={() => refetch()}
        />
      )}

      {!isLoading && documents.length === 0 && <EmptyState message="No documents found." />}

      {groups.map((group) => (
        <Panel
          key={group.prefix || "__uncategorized"}
          header={<span className={styles.sectionTitle}>{group.prefix || "Uncategorized"}</span>}
        >
          <ul className={styles.docList}>
            {group.documents.map((doc) => (
              <DocumentRow key={doc.document_id} doc={doc} />
            ))}
          </ul>
        </Panel>
      ))}

      {hasNextPage && (
        <div className={styles.center}>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => fetchNextPage()}
            disabled={isFetchingNextPage}
          >
            {isFetchingNextPage ? "Loading..." : "Load more"}
          </Button>
        </div>
      )}

      <DocumentCreateModal
        open={createOpen}
        onClose={() => setCreateOpen(false)}
      />
    </div>
  );
}
