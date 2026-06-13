import { useCallback, useEffect, useMemo, useState } from "react";
import { Button, Icons, Spinner } from "@hydra/ui";
import { DocumentCreateModal } from "../features/documents/DocumentCreateModal";
import {
  DocumentTreeBranch,
  isFolderEntry,
} from "../features/documents/DocumentTree";
import { DocumentsReaderPane } from "../features/documents/DocumentsReaderPane";
import { useDocumentTreeExpandState } from "../features/documents/useDocumentTreeExpandState";
import { useBatchedDocumentPaths } from "../features/documents/useBatchedDocumentPaths";
import { useUncategorizedDocuments } from "../features/documents/useUncategorizedDocuments";
import { useMediaQuery } from "../hooks/useMediaQuery";
import { useBreadcrumbs } from "../layout/useBreadcrumbs";
import { PageHead } from "../layout/PageHead";
import styles from "./DocumentsPage.module.css";

const ROOT_PATH = "/";
const MOBILE_MEDIA_QUERY = "(max-width: 768px)";

export function DocumentsPage() {
  useBreadcrumbs([{ label: "Workspace", to: "/" }], "Documents");
  const [createOpen, setCreateOpen] = useState(false);
  const [activePath, setActivePath] = useState<string>(ROOT_PATH);
  const [treeOpen, setTreeOpen] = useState(false);
  const isMobile = useMediaQuery(MOBILE_MEDIA_QUERY);

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

  const topLevelEntries = useMemo(() => childrenMap.get(ROOT_PATH) ?? [], [childrenMap]);
  const topLevelFolders = useMemo(() => topLevelEntries.filter(isFolderEntry), [topLevelEntries]);
  const topLevelPaths = useMemo(() => topLevelFolders.map((c) => c.full_path), [topLevelFolders]);

  useEffect(() => {
    autoExpand(topLevelPaths);
  }, [topLevelPaths, autoExpand]);

  // Snap the drawer closed when crossing into desktop; otherwise its open state
  // lingers and the inline tree renders on top of a stale `treeOpen=true`.
  useEffect(() => {
    if (!isMobile) setTreeOpen(false);
  }, [isMobile]);

  useEffect(() => {
    if (!treeOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setTreeOpen(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [treeOpen]);

  const onSelectPath = useCallback(
    (path: string) => {
      setActivePath(path);
      setTreeOpen(false);
    },
    [],
  );

  const totalDocs = topLevelEntries.length + (uncategorized?.documents.length ?? 0);

  const drawerActive = isMobile && treeOpen;

  return (
    <div className={styles.page}>
      {isMobile ? (
        <h1 className={styles.visuallyHiddenTitle}>Documents</h1>
      ) : (
        <PageHead
          title="Documents"
          actions={
            <Button variant="primary" size="sm" onClick={() => setCreateOpen(true)}>
              <Icons.IconPlus />
              New document
            </Button>
          }
        />
      )}

      {error && <div className={styles.errorBanner}>Failed to load documents: {error.message}</div>}

      {isLoading && (
        <div className={styles.center}>
          <Spinner size="md" />
        </div>
      )}

      {!isLoading && (
        <div className={styles.treeLayout}>
          {drawerActive && (
            <div
              className={styles.drawerBackdrop}
              onClick={() => setTreeOpen(false)}
              aria-hidden="true"
              data-testid="documents-tree-backdrop"
            />
          )}
          <aside
            className={`${styles.tree}${drawerActive ? ` ${styles.treeDrawerOpen}` : ""}`}
            aria-label="Document tree"
            aria-hidden={isMobile && !treeOpen ? true : undefined}
          >
            <ul className={styles.treeRoot} role="tree">
              <li>
                <div
                  className={`${styles.folderRow}${activePath === ROOT_PATH ? ` ${styles.folderRowActive}` : ""}`}
                  onClick={() => onSelectPath(ROOT_PATH)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === " ") {
                      e.preventDefault();
                      onSelectPath(ROOT_PATH);
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
                  onSelect={onSelectPath}
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
            onSelectFolder={onSelectPath}
            getChildren={getChildren}
            pathsLoading={isFetching}
            onOpenTree={isMobile ? () => setTreeOpen(true) : undefined}
            onCreate={isMobile ? () => setCreateOpen(true) : undefined}
          />
        </div>
      )}

      <DocumentCreateModal open={createOpen} onClose={() => setCreateOpen(false)} />
    </div>
  );
}
