import { useState, useMemo, useEffect } from "react";
import { Button, Icons, Spinner } from "@hydra/ui";
import { DocumentCreateModal } from "../features/documents/DocumentCreateModal";
import {
  DocumentTreeBranch,
  isFolderEntry,
} from "../features/documents/DocumentTree";
import { DocumentsReaderPane } from "../features/documents/DocumentsReaderPane";
import { useDocumentTreeExpandState } from "../features/documents/useDocumentTreeExpandState";
import { useBatchedDocumentPaths } from "../features/documents/useBatchedDocumentPaths";
import { useDocumentCount } from "../features/documents/useDocumentCount";
import { useUncategorizedDocuments } from "../features/documents/useUncategorizedDocuments";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import { PageHead } from "../layout/PageHead";
import { FloatingActionButton } from "../layout/FloatingActionButton";
import styles from "./DocumentsPage.module.css";

const ROOT_PATH = "/";

export function DocumentsPage() {
  useBreadcrumbs([{ label: "Workspace", to: "/" }], "Documents");
  const [createOpen, setCreateOpen] = useState(false);
  const [activePath, setActivePath] = useState<string>(ROOT_PATH);

  const { expandedPaths, onToggle, autoExpand } = useDocumentTreeExpandState();

  const prefixes = useMemo<(string | null)[]>(() => {
    const set = new Set<string>();
    set.add(ROOT_PATH);
    for (const p of expandedPaths) set.add(p);
    if (activePath !== ROOT_PATH) set.add(activePath);
    return [...set];
  }, [expandedPaths, activePath]);

  const { childrenMap, getChildren, isLoading, isFetching, error } =
    useBatchedDocumentPaths(prefixes);

  const { data: uncategorized } = useUncategorizedDocuments(true);
  const { data: totalCount } = useDocumentCount();

  const topLevelEntries = useMemo(() => childrenMap.get(ROOT_PATH) ?? [], [childrenMap]);
  const topLevelFolders = useMemo(() => topLevelEntries.filter(isFolderEntry), [topLevelEntries]);
  const topLevelPaths = useMemo(() => topLevelFolders.map((c) => c.full_path), [topLevelFolders]);

  useEffect(() => {
    autoExpand(topLevelPaths);
  }, [topLevelPaths, autoExpand]);

  const totalDocs = topLevelEntries.length + (uncategorized?.documents.length ?? 0);
  const displayCount = totalCount ?? totalDocs;
  const totalLabel = displayCount === 1 ? "1 DOC" : `${displayCount} DOCS`;

  return (
    <div className={styles.page}>
      <PageHead
        eyebrow={`KNOWLEDGE · ${totalLabel}`}
        title="Documents"
        actions={
          <Button variant="primary" size="sm" onClick={() => setCreateOpen(true)}>
            <Icons.IconPlus />
            New document
          </Button>
        }
      />

      {error && <div className={styles.errorBanner}>Failed to load documents: {error.message}</div>}

      {isLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}

      {!isLoading && (
        <div className={styles.treeLayout}>
          <aside className={styles.tree} aria-label="Document tree">
            <ul className={styles.treeRoot} role="tree">
              <li>
                <div
                  className={`${styles.folderRow}${activePath === ROOT_PATH ? ` ${styles.folderRowActive}` : ""}`}
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
                <DocumentTreeBranch
                  key={entry.full_path}
                  entry={entry}
                  depth={1}
                  activePath={activePath}
                  onSelect={setActivePath}
                  expandedPaths={expandedPaths}
                  onToggleExpand={onToggle}
                  getChildren={getChildren}
                  isFetching={isFetching}
                />
              ))}
            </ul>
          </aside>

          <DocumentsReaderPane
            activePath={activePath}
            onSelectFolder={setActivePath}
            getChildren={getChildren}
            pathsLoading={isFetching}
          />
        </div>
      )}

      <DocumentCreateModal open={createOpen} onClose={() => setCreateOpen(false)} />
      <FloatingActionButton
        icon={<Icons.IconPlus size={24} />}
        label="New document"
        onClick={() => setCreateOpen(true)}
        testId="documents-fab"
      />
    </div>
  );
}
