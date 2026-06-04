import { useCallback, useMemo } from "react";
import { Link } from "react-router-dom";
import { Icons, Spinner } from "@hydra/ui";
import type { PathChildEntry } from "@hydra/api";
import styles from "./DocumentTree.module.css";

/** Whether an entry renders as a folder branch (has children — possibly also a doc). */
export function isFolderEntry(entry: PathChildEntry): boolean {
  if (!entry.is_document) return true;
  return Number(entry.child_count) > 1;
}

/** Whether an entry is a leaf document (a document with no further descendants). */
export function isLeafDocumentEntry(entry: PathChildEntry): boolean {
  return entry.is_document && Number(entry.child_count) <= 1;
}

interface DocumentTreeBranchProps {
  entry: PathChildEntry;
  depth: number;
  activePath: string;
  onSelect: (path: string) => void;
  expandedPaths: Set<string>;
  onToggleExpand: (path: string) => void;
  getChildren: (prefix: string | null) => PathChildEntry[];
  isFetching: boolean;
}

export function DocumentTreeBranch({
  entry,
  depth,
  activePath,
  onSelect,
  expandedPaths,
  onToggleExpand,
  getChildren,
  isFetching,
}: DocumentTreeBranchProps) {
  const expanded = expandedPaths.has(entry.full_path);
  const isActive = activePath === entry.full_path;

  const children = useMemo(
    () => (expanded ? getChildren(entry.full_path) : []),
    [expanded, getChildren, entry.full_path],
  );

  const folderChildren = useMemo(() => children.filter(isFolderEntry), [children]);
  const leafDocChildren = useMemo(() => children.filter(isLeafDocumentEntry), [children]);

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
          {isFetching && children.length === 0 && (
            <li className={styles.loadingRow}>
              <Spinner size="sm" />
            </li>
          )}
          {folderChildren.map((child) => (
            <DocumentTreeBranch
              key={child.full_path}
              entry={child}
              depth={depth + 1}
              activePath={activePath}
              onSelect={onSelect}
              expandedPaths={expandedPaths}
              onToggleExpand={onToggleExpand}
              getChildren={getChildren}
              isFetching={isFetching}
            />
          ))}
          {leafDocChildren.map((child) => (
            <DocumentTreeLeaf key={child.full_path} entry={child} depth={depth + 1} />
          ))}
        </ul>
      )}
    </li>
  );
}

interface DocumentTreeLeafProps {
  entry: PathChildEntry;
  depth: number;
}

export function DocumentTreeLeaf({ entry, depth }: DocumentTreeLeafProps) {
  const padding = { paddingLeft: `${8 + depth * 14}px` } as const;
  const docId = entry.document?.document_id;
  if (!docId) {
    return (
      <li>
        <div className={styles.folderRow} style={padding} title={entry.name} aria-disabled="true">
          <span className={styles.chevronPlaceholder} />
          <span className={styles.folderIcon}>
            <Icons.IconDoc size={14} />
          </span>
          <span className={styles.folderName}>{entry.name}</span>
        </div>
      </li>
    );
  }
  const title = entry.document?.title || entry.name;
  return (
    <li>
      <Link
        to={`/documents/${docId}`}
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
