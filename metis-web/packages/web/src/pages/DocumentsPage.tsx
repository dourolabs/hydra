import { useState, useMemo, useCallback } from "react";
import { Link } from "react-router-dom";
import { useMutation, useQueryClient, useInfiniteQuery } from "@tanstack/react-query";
import { Panel, Spinner, Button, Modal, Input, Textarea } from "@metis/ui";
import type { DocumentSummaryRecord, ListDocumentsResponse } from "@metis/api";
import { apiClient } from "../api/client";
import { useToast } from "../features/toast/useToast";
import { formatRelativeTime } from "../utils/time";
import styles from "./DocumentsPage.module.css";

const PAGE_SIZE = 50;

interface DocumentGroup {
  prefix: string;
  documents: DocumentSummaryRecord[];
}

function getPathPrefix(doc: DocumentSummaryRecord): string {
  const path = doc.document.path;
  if (!path) return "";
  // Strip leading slash, then take the first path segment
  const cleaned = path.startsWith("/") ? path.slice(1) : path;
  const slashIndex = cleaned.indexOf("/");
  if (slashIndex < 0) return "";
  return cleaned.slice(0, slashIndex);
}

function groupDocumentsByPrefix(documents: DocumentSummaryRecord[]): DocumentGroup[] {
  const groups = new Map<string, DocumentSummaryRecord[]>();

  for (const doc of documents) {
    if (doc.document.deleted) continue;
    const prefix = getPathPrefix(doc);
    const list = groups.get(prefix) ?? [];
    list.push(doc);
    groups.set(prefix, list);
  }

  // Sort groups alphabetically, with uncategorized ("") last
  const sorted: DocumentGroup[] = [];
  const keys = Array.from(groups.keys()).sort((a, b) => {
    if (a === "") return 1;
    if (b === "") return -1;
    return a.localeCompare(b);
  });

  for (const key of keys) {
    sorted.push({ prefix: key, documents: groups.get(key)! });
  }

  return sorted;
}

function getDocumentDisplayTitle(doc: DocumentSummaryRecord): string {
  if (doc.document.title) return doc.document.title;
  if (doc.document.path) return doc.document.path;
  return doc.document_id;
}

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

      {isLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}

      {error && (
        <p className={styles.error}>Failed to load documents: {(error as Error).message}</p>
      )}

      {!isLoading && documents.length === 0 && <p className={styles.empty}>No documents found.</p>}

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

interface DocumentRowProps {
  doc: DocumentSummaryRecord;
}

function DocumentRow({ doc }: DocumentRowProps) {
  const [deleteOpen, setDeleteOpen] = useState(false);
  const { addToast } = useToast();
  const queryClient = useQueryClient();

  const deleteMutation = useMutation({
    mutationFn: () => apiClient.deleteDocument(doc.document_id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["paginatedDocuments"] });
      queryClient.invalidateQueries({ queryKey: ["documents"] });
      addToast("Document deleted", "success");
      setDeleteOpen(false);
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to delete document",
        "error",
      );
    },
  });

  return (
    <li className={styles.docRow}>
      <Link
        to={`/documents/${doc.document_id}`}
        className={styles.docRowLink}
      >
        <span className={styles.docTitle}>{getDocumentDisplayTitle(doc)}</span>
        <div className={styles.docMeta}>
          {doc.document.path && <span className={styles.docPath}>{doc.document.path}</span>}
          <span className={styles.docTime}>{formatRelativeTime(doc.timestamp)}</span>
        </div>
      </Link>
      <Button
        variant="ghost"
        size="sm"
        className={styles.deleteButton}
        onClick={(e) => {
          e.stopPropagation();
          e.preventDefault();
          setDeleteOpen(true);
        }}
        aria-label="Delete document"
      >
        Delete
      </Button>
      <DocumentDeleteModal
        open={deleteOpen}
        onClose={() => {
          if (!deleteMutation.isPending) setDeleteOpen(false);
        }}
        onConfirm={() => deleteMutation.mutate()}
        isPending={deleteMutation.isPending}
        doc={doc}
      />
    </li>
  );
}

interface DocumentDeleteModalProps {
  open: boolean;
  onClose: () => void;
  onConfirm: () => void;
  isPending: boolean;
  doc: DocumentSummaryRecord;
}

function DocumentDeleteModal({ open, onClose, onConfirm, isPending, doc }: DocumentDeleteModalProps) {
  return (
    <Modal open={open} onClose={onClose} title="Delete Document">
      <div className={styles.deleteModalContent}>
        <p className={styles.deleteMessage}>
          Are you sure you want to delete this document?
        </p>
        <p className={styles.deleteDocTitle}>{getDocumentDisplayTitle(doc)}</p>
        {doc.document.path && (
          <p className={styles.deleteDocPath}>{doc.document.path}</p>
        )}
        <div className={styles.deleteActions}>
          <Button variant="secondary" size="md" onClick={onClose} disabled={isPending}>
            Cancel
          </Button>
          <Button variant="danger" size="md" onClick={onConfirm} disabled={isPending}>
            {isPending ? "Deleting..." : "Delete"}
          </Button>
        </div>
      </div>
    </Modal>
  );
}

interface DocumentCreateModalProps {
  open: boolean;
  onClose: () => void;
}

function DocumentCreateModal({ open, onClose }: DocumentCreateModalProps) {
  const { addToast } = useToast();
  const queryClient = useQueryClient();

  const [title, setTitle] = useState("");
  const [path, setPath] = useState("");
  const [bodyMarkdown, setBodyMarkdown] = useState("");

  const resetForm = useCallback(() => {
    setTitle("");
    setPath("");
    setBodyMarkdown("");
  }, []);

  const mutation = useMutation({
    mutationFn: (params: { title: string; path: string; body_markdown: string }) =>
      apiClient.createDocument({
        document: {
          title: params.title,
          body_markdown: params.body_markdown,
          ...(params.path ? { path: params.path } : {}),
        },
      }),
    onSuccess: (data) => {
      resetForm();
      queryClient.invalidateQueries({ queryKey: ["paginatedDocuments"] });
      queryClient.invalidateQueries({ queryKey: ["documents"] });
      addToast(`Document ${data.document_id} created`, "success");
      onClose();
    },
    onError: (err) => {
      addToast(
        err instanceof Error ? err.message : "Failed to create document",
        "error",
      );
    },
  });

  const isValid = title.trim().length > 0 && (!path || path.startsWith("/"));

  const handleSubmit = useCallback(() => {
    if (!isValid) return;
    mutation.mutate({
      title: title.trim(),
      path: path.trim(),
      body_markdown: bodyMarkdown,
    });
  }, [title, path, bodyMarkdown, isValid, mutation]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        handleSubmit();
      }
    },
    [handleSubmit],
  );

  const handleClose = useCallback(() => {
    if (!mutation.isPending) {
      resetForm();
      onClose();
    }
  }, [mutation.isPending, resetForm, onClose]);

  return (
    <Modal open={open} onClose={handleClose} title="New Document" className={styles.createModal}>
      <div className={styles.createForm} onKeyDown={handleKeyDown}>
        <div className={styles.createFields}>
          <Input
            label="Title"
            placeholder="Document title"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            required
          />
          <Input
            label="Path"
            placeholder="/path/to/document.md"
            value={path}
            onChange={(e) => setPath(e.target.value)}
            error={path && !path.startsWith("/") ? "Path must start with /" : undefined}
          />
        </div>
        <div className={styles.createBodyWrapper}>
          <Textarea
            label="Body"
            placeholder="Markdown content..."
            value={bodyMarkdown}
            onChange={(e) => setBodyMarkdown(e.target.value)}
            className={styles.createBodyTextarea}
          />
        </div>
        <div className={styles.createFooter}>
          <span className={styles.createHint}>
            {navigator.platform.includes("Mac") ? "\u2318" : "Ctrl"}+Enter to create
          </span>
          <div className={styles.createActions}>
            <Button variant="secondary" size="md" onClick={handleClose}>
              Cancel
            </Button>
            <Button
              variant="primary"
              size="md"
              onClick={handleSubmit}
              disabled={!isValid || mutation.isPending}
            >
              {mutation.isPending ? "Creating..." : "Create Document"}
            </Button>
          </div>
        </div>
      </div>
    </Modal>
  );
}
