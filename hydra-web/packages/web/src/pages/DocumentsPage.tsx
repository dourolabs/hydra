import { useState, useMemo, useCallback } from "react";
import { Link } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { Button, Icons, Spinner } from "@hydra/ui";
import type {
  DocumentSummaryRecord,
  ListDocumentPathsResponse,
  ListDocumentsResponse,
  PathChildEntry,
} from "@hydra/api";
import { apiClient } from "../api/client";
import { DocumentCreateModal } from "../features/documents/DocumentCreateModal";
import { useDocumentTreeExpandState } from "../features/documents/useDocumentTreeExpandState";
import { useDocumentSummariesUnderPath } from "../features/documents/useDocumentSummariesUnderPath";
import { getDocumentDisplayTitle } from "../features/documents/utils";
import { formatRelativeTime } from "../utils/time";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import styles from "./DocumentsPage.module.css";

const ROOT_PATH = "/";

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
    queryFn: () => apiClient.listDocuments({ path_prefix: path, path_is_exact: true }),
    enabled,
  });
}

function useUncategorizedDocuments(enabled: boolean) {
  return useQuery<ListDocumentsResponse, Error>({
    queryKey: ["uncategorizedDocuments"],
    queryFn: () => apiClient.listDocuments({ limit: 200 }),
    select: (data) => ({
      ...data,
      documents: data.documents.filter((d) => !d.document.path && !d.document.deleted),
    }),
    enabled,
  });
}

/** Whether an entry renders as a folder branch (has children — possibly also a doc). */
function isFolderEntry(entry: PathChildEntry): boolean {
  if (!entry.is_document) return true;
  return Number(entry.child_count) > 1;
}

/** Whether an entry is a leaf document (a document with no further descendants). */
function isLeafDocumentEntry(entry: PathChildEntry): boolean {
  return entry.is_document && Number(entry.child_count) <= 1;
}

interface TreeBranchProps {
  entry: PathChildEntry;
  depth: number;
  activePath: string;
  onSelect: (path: string) => void;
  expandedPaths: Set<string>;
  onToggleExpand: (path: string) => void;
}

function TreeBranch({
  entry,
  depth,
  activePath,
  onSelect,
  expandedPaths,
  onToggleExpand,
}: TreeBranchProps) {
  const expanded = expandedPaths.has(entry.full_path);
  const isActive = activePath === entry.full_path;

  const { data: childPaths, isLoading } = useDocumentPaths(entry.full_path, expanded);
  const { data: docsUnder } = useDocumentSummariesUnderPath(
    entry.full_path,
    expanded,
  );

  const folderChildren = useMemo(
    () => (childPaths?.children ?? []).filter(isFolderEntry),
    [childPaths],
  );

  const leafDocChildren = useMemo(
    () => (childPaths?.children ?? []).filter(isLeafDocumentEntry),
    [childPaths],
  );

  const pathToDoc = useMemo(() => {
    const map = new Map<string, DocumentSummaryRecord>();
    for (const record of docsUnder?.documents ?? []) {
      if (record.document.deleted) continue;
      const p = record.document.path;
      if (p == null) continue;
      if (!map.has(p)) map.set(p, record);
    }
    return map;
  }, [docsUnder]);

  const handleChevron = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation();
      onToggleExpand(entry.full_path);
    },
    [entry.full_path, onToggleExpand],
  );

  const handleSelect = useCallback(() => {
    onSelect(entry.full_path);
    if (!expanded) onToggleExpand(entry.full_path);
  }, [entry.full_path, onSelect, expanded, onToggleExpand]);

  return (
    <li>
      <div
        className={`${styles.folderRow}${isActive ? ` ${styles.folderRowActive}` : ""}`}
        style={{ paddingLeft: `${8 + depth * 14}px` }}
        onClick={handleSelect}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            handleSelect();
          }
        }}
        role="treeitem"
        aria-expanded={expanded}
        aria-selected={isActive}
        tabIndex={0}
      >
        <button
          type="button"
          className={`${styles.chevron}${expanded ? ` ${styles.chevronOpen}` : ""}`}
          onClick={handleChevron}
          aria-label={expanded ? "Collapse" : "Expand"}
          tabIndex={-1}
        >
          <Icons.IconChevronRight size={12} />
        </button>
        <span className={styles.folderIcon}>
          <Icons.IconFolder size={14} />
        </span>
        <span className={styles.folderName}>{entry.name}</span>
        <span className={styles.fileCount}>{Number(entry.child_count)}</span>
      </div>
      {expanded && (
        <ul className={styles.treeRoot}>
          {isLoading && (
            <li className={styles.loadingRow}>
              <Spinner size="sm" />
            </li>
          )}
          {folderChildren.map((child) => (
            <TreeBranch
              key={child.full_path}
              entry={child}
              depth={depth + 1}
              activePath={activePath}
              onSelect={onSelect}
              expandedPaths={expandedPaths}
              onToggleExpand={onToggleExpand}
            />
          ))}
          {leafDocChildren.map((child) => (
            <TreeDocLeaf
              key={child.full_path}
              entry={child}
              depth={depth + 1}
              doc={pathToDoc.get(child.full_path)}
            />
          ))}
        </ul>
      )}
    </li>
  );
}

interface TreeDocLeafProps {
  entry: PathChildEntry;
  depth: number;
  doc: DocumentSummaryRecord | undefined;
}

function TreeDocLeaf({ entry, depth, doc }: TreeDocLeafProps) {
  const padding = { paddingLeft: `${8 + depth * 14}px` } as const;
  if (!doc) {
    return (
      <li>
        <div
          className={styles.folderRow}
          style={padding}
          title={entry.name}
          aria-disabled="true"
        >
          <span className={styles.chevronPlaceholder} />
          <span className={styles.folderIcon}>
            <Icons.IconDoc size={14} />
          </span>
          <span className={styles.folderName}>{entry.name}</span>
        </div>
      </li>
    );
  }
  const title = getDocumentDisplayTitle(doc);
  return (
    <li>
      <Link
        to={`/documents/${doc.document_id}`}
        className={`${styles.folderRow} ${styles.docLeafLink}`}
        style={padding}
        role="treeitem"
        title={title}
      >
        <span className={styles.chevronPlaceholder} />
        <span className={styles.folderIcon}>
          <Icons.IconDoc size={14} />
        </span>
        <span className={styles.folderName}>{title}</span>
      </Link>
    </li>
  );
}

interface BreadcrumbItem {
  name: string;
  path: string;
}

function pathBreadcrumbs(activePath: string): BreadcrumbItem[] {
  if (activePath === ROOT_PATH) return [];
  const segs = activePath.split("/").filter(Boolean);
  const out: BreadcrumbItem[] = [];
  let cur = "";
  for (const s of segs) {
    cur += "/" + s;
    out.push({ name: s, path: cur });
  }
  return out;
}

interface ReaderPaneProps {
  activePath: string;
  onSelectFolder: (path: string) => void;
}

function ReaderPane({ activePath, onSelectFolder }: ReaderPaneProps) {
  const isRoot = activePath === ROOT_PATH;
  const prefix = isRoot ? null : activePath;

  const { data: childPaths } = useDocumentPaths(prefix, true);
  const { data: docsAtPath, isLoading: docsLoading } = useDocumentsAtPath(
    activePath,
    !isRoot,
  );
  const { data: rootDocs, isLoading: rootDocsLoading } = useUncategorizedDocuments(isRoot);

  const subfolders = useMemo(
    () => (childPaths?.children ?? []).filter(isFolderEntry),
    [childPaths],
  );

  const docs: DocumentSummaryRecord[] = useMemo(() => {
    if (isRoot) {
      return (rootDocs?.documents ?? []).filter((d) => !d.document.deleted);
    }
    return (docsAtPath?.documents ?? []).filter((d) => !d.document.deleted);
  }, [isRoot, docsAtPath, rootDocs]);

  const breadcrumbs = pathBreadcrumbs(activePath);
  const isLoading = isRoot ? rootDocsLoading : docsLoading;
  const totalFolders = subfolders.length;
  const totalFiles = docs.length;

  return (
    <div className={styles.pane}>
      <div className={styles.breadcrumb}>
        {breadcrumbs.map((b, i) => {
          const isLast = i === breadcrumbs.length - 1;
          return (
            <span key={b.path}>
              {i > 0 && <span className={styles.crumbSep}>/</span>}
              <span
                className={isLast ? styles.crumbCurrent : styles.crumb}
                onClick={isLast ? undefined : () => onSelectFolder(b.path)}
              >
                {b.name}
              </span>
            </span>
          );
        })}
        <span className={styles.crumbSpacer} />
        <span className={styles.crumbMeta}>
          {totalFiles} {totalFiles === 1 ? "file" : "files"} · {totalFolders}{" "}
          {totalFolders === 1 ? "folder" : "folders"}
        </span>
      </div>

      <div className={styles.paneBody}>
        {isLoading && totalFiles === 0 && totalFolders === 0 && (
          <div className={styles.center}>
            <Spinner size="md" />
          </div>
        )}

        {!isLoading && totalFiles === 0 && totalFolders === 0 && (
          <div className={styles.empty}>This folder is empty.</div>
        )}

        {subfolders.map((f) => (
          <div
            key={f.full_path}
            className={styles.docRow}
            onClick={() => onSelectFolder(f.full_path)}
            role="button"
            tabIndex={0}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onSelectFolder(f.full_path);
              }
            }}
          >
            <span className={styles.docRowIcon}>
              <Icons.IconFolder size={14} />
            </span>
            <span className={styles.docRowTitle}>{f.name}</span>
            <span className={styles.docRowMeta}>
              {Number(f.child_count)} {Number(f.child_count) === 1 ? "file" : "files"}
            </span>
          </div>
        ))}

        {docs.map((doc) => (
          <Link
            key={doc.document_id}
            to={`/documents/${doc.document_id}`}
            className={styles.docRow}
          >
            <span className={styles.docRowIcon}>
              <Icons.IconDoc size={14} />
            </span>
            <span className={styles.docRowTitle}>{getDocumentDisplayTitle(doc)}</span>
            <span className={styles.docRowDate}>{formatRelativeTime(doc.timestamp)}</span>
          </Link>
        ))}
      </div>
    </div>
  );
}

export function DocumentsPage() {
  useBreadcrumbs([{ label: "Workspace", to: "/" }], "Documents");
  const [createOpen, setCreateOpen] = useState(false);
  const [activePath, setActivePath] = useState<string>(ROOT_PATH);

  const { data: topLevel, isLoading, error } = useDocumentPaths(null, true);
  const { data: uncategorized } = useUncategorizedDocuments(true);

  const topLevelFolders = useMemo(
    () => (topLevel?.children ?? []).filter(isFolderEntry),
    [topLevel],
  );

  const topLevelPaths = useMemo(
    () => topLevelFolders.map((c) => c.full_path),
    [topLevelFolders],
  );

  const { expandedPaths, onToggle } = useDocumentTreeExpandState(topLevelPaths);

  const totalDocs =
    (topLevel?.children.length ?? 0) + (uncategorized?.documents.length ?? 0);
  const totalLabel = totalDocs === 1 ? "1 DOC" : `${totalDocs} DOCS`;

  return (
    <div className={styles.page}>
      <div className={styles.pageHead}>
        <div className={styles.headLeft}>
          <span className={styles.eyebrow}>KNOWLEDGE · {totalLabel}</span>
          <h1 className={styles.pageTitle}>Documents</h1>
        </div>
        <span className={styles.headSpacer} />
        <Button variant="primary" size="sm" onClick={() => setCreateOpen(true)}>
          <Icons.IconPlus />
          New document
        </Button>
      </div>

      {error && (
        <div className={styles.errorBanner}>
          Failed to load documents: {error.message}
        </div>
      )}

      {isLoading && !topLevel && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}

      {topLevel && (
        <div className={styles.treeLayout}>
          <aside className={styles.tree} aria-label="Document tree">
            <ul className={styles.treeRoot} role="tree">
              <li>
                <div
                  className={`${styles.folderRow}${activePath === ROOT_PATH ? ` ${styles.folderRowActive}` : ""}`}
                  style={{ paddingLeft: "8px" }}
                  onClick={() => setActivePath(ROOT_PATH)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      setActivePath(ROOT_PATH);
                    }
                  }}
                  role="treeitem"
                  aria-selected={activePath === ROOT_PATH}
                  tabIndex={0}
                >
                  <span className={styles.chevronPlaceholder} />
                  <span className={styles.folderIcon}>
                    <Icons.IconFolder size={14} />
                  </span>
                  <span className={styles.folderName}>/</span>
                  <span className={styles.fileCount}>{totalDocs}</span>
                </div>
              </li>
              {topLevelFolders.map((entry) => (
                <TreeBranch
                  key={entry.full_path}
                  entry={entry}
                  depth={1}
                  activePath={activePath}
                  onSelect={setActivePath}
                  expandedPaths={expandedPaths}
                  onToggleExpand={onToggle}
                />
              ))}
            </ul>
          </aside>

          <ReaderPane activePath={activePath} onSelectFolder={setActivePath} />
        </div>
      )}

      <DocumentCreateModal open={createOpen} onClose={() => setCreateOpen(false)} />
    </div>
  );
}
