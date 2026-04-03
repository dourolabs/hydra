import { useState, useCallback } from "react";
import { useQuery } from "@tanstack/react-query";
import { Panel, Button, Spinner } from "@hydra/ui";
import type {
  ListDocumentPathsResponse,
  ListDocumentsResponse,
  PathChildEntry,
} from "@hydra/api";
import { apiClient } from "../api/client";
import { LoadingState } from "../components/LoadingState/LoadingState";
import { ErrorState } from "../components/ErrorState/ErrorState";
import { EmptyState } from "../components/EmptyState/EmptyState";
import { DocumentRow } from "../features/documents/DocumentRow";
import { DocumentCreateModal } from "../features/documents/DocumentCreateModal";
import styles from "./DocumentsPage.module.css";

function useDocumentPaths(prefix: string | null, enabled: boolean) {
  return useQuery<ListDocumentPathsResponse, Error>({
    queryKey: ["documentPaths", prefix],
    queryFn: () => apiClient.listDocumentPaths({ prefix }),
    enabled,
  });
}

function useDocumentsAtPath(path: string, enabled: boolean) {
  return useQuery<ListDocumentsResponse, Error>({
    queryKey: ["documentsAtPath", path],
    queryFn: () =>
      apiClient.listDocuments({ path_prefix: path, path_is_exact: true }),
    enabled,
  });
}

function useUncategorizedDocuments() {
  return useQuery<ListDocumentsResponse, Error>({
    queryKey: ["uncategorizedDocuments"],
    queryFn: () => apiClient.listDocuments({ limit: 200 }),
    select: (data) => ({
      ...data,
      documents: data.documents.filter(
        (d) => !d.document.path && !d.document.deleted,
      ),
    }),
  });
}

interface FolderNodeProps {
  entry: PathChildEntry;
  depth: number;
}

function FolderNode({ entry, depth }: FolderNodeProps) {
  const [expanded, setExpanded] = useState(false);

  const { data: childPaths, isLoading: loadingPaths } = useDocumentPaths(
    entry.full_path,
    expanded,
  );

  const hasChildren = childPaths && childPaths.children.length > 0;
  const isLeaf = childPaths && childPaths.children.length === 0;

  const { data: leafDocs, isLoading: loadingDocs } = useDocumentsAtPath(
    entry.full_path,
    expanded && isLeaf === true,
  );

  const toggle = useCallback(() => setExpanded((prev) => !prev), []);

  return (
    <li className={styles.treeNode}>
      <button
        className={styles.folderRow}
        style={{ paddingLeft: `calc(${depth} * var(--space-6) + var(--space-3))` }}
        onClick={toggle}
        aria-expanded={expanded}
      >
        <span className={styles.chevron}>{expanded ? "\u25BC" : "\u25B6"}</span>
        <span className={styles.folderName}>{entry.name}</span>
        <span className={styles.childCount}>{Number(entry.child_count)}</span>
      </button>
      {expanded && (
        <ul className={styles.treeChildren}>
          {(loadingPaths || loadingDocs) && (
            <li className={styles.loadingRow}>
              <Spinner size="sm" />
            </li>
          )}
          {hasChildren &&
            childPaths.children.map((child) => (
              <FolderNode
                key={child.full_path}
                entry={child}
                depth={depth + 1}
              />
            ))}
          {isLeaf &&
            leafDocs?.documents
              .filter((d) => !d.document.deleted)
              .map((doc) => (
                <DocumentRow key={doc.document_id} doc={doc} />
              ))}
        </ul>
      )}
    </li>
  );
}

export function DocumentsPage() {
  const [createOpen, setCreateOpen] = useState(false);

  const {
    data: topLevel,
    isLoading,
    error,
    refetch,
  } = useDocumentPaths(null, true);

  const { data: uncategorized, isLoading: loadingUncategorized } =
    useUncategorizedDocuments();

  const hasTopLevel = topLevel && topLevel.children.length > 0;
  const hasUncategorized =
    uncategorized && uncategorized.documents.length > 0;
  const isEmpty =
    !isLoading &&
    !loadingUncategorized &&
    !hasTopLevel &&
    !hasUncategorized;

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
          message={`Failed to load documents: ${error.message}`}
          onRetry={() => refetch()}
        />
      )}

      {isEmpty && <EmptyState message="No documents found." />}

      {hasTopLevel && (
        <Panel
          header={
            <span className={styles.sectionTitle}>Documents</span>
          }
        >
          <ul className={styles.treeRoot}>
            {topLevel.children.map((entry) => (
              <FolderNode key={entry.full_path} entry={entry} depth={0} />
            ))}
          </ul>
        </Panel>
      )}

      {hasUncategorized && (
        <Panel
          header={
            <span className={styles.sectionTitle}>Uncategorized</span>
          }
        >
          <ul className={styles.docList}>
            {uncategorized.documents.map((doc) => (
              <DocumentRow key={doc.document_id} doc={doc} />
            ))}
          </ul>
        </Panel>
      )}

      <DocumentCreateModal
        open={createOpen}
        onClose={() => setCreateOpen(false)}
      />
    </div>
  );
}
